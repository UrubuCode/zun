# Function pointers gerenciados e não gerenciados (delegate*)

## Mecanismo no .NET

O C# 9 expõe `delegate*<int, void>` (gerenciado) e `delegate* unmanaged[Cdecl]<int, void>` (não gerenciado). Eles são compilados para o tipo de assinatura "function pointer" da ECMA-335 e usados com a instrução IL `calli`. Nenhum objeto é alocado: o valor é apenas um endereço (`native int`).

### Representação no runtime

- O tipo é representado por `FnPtrTypeDesc` (`src/coreclr/vm/typedesc.h`, linha ~428), um `TypeDesc` — como ponteiros e byrefs, não um `MethodTable` de classe.
- A convenção de chamada vive na assinatura, em `CorInfoCallConv` (`src/coreclr/inc/corinfo.h`, linha 651). Valores confirmados: `CORINFO_CALLCONV_DEFAULT = 0x0` (gerenciado), `CORINFO_CALLCONV_VARARG = 0x5`, `CORINFO_CALLCONV_UNMANAGED = 0x9`, `CORINFO_CALLCONV_NATIVEVARARG = 0xb` ("usado APENAS para chamadas vararg PInvoke em IL stub"). Os valores C/STDCALL/THISCALL/FASTCALL clássicos estão **comentados** no enum: a escolha concreta (Cdecl, Stdcall, ...) é carregada como *modifiers* na assinatura (`modopt`), não no byte base.
- `CallConvBuilder` (`src/coreclr/vm/callconvbuilder.hpp/.cpp`) acumula `CallConvBase` + `CallConvModifiers` a partir dos `modopt` `CallConvCdecl`, `CallConvSuppressGCTransition` etc. e do atributo `UnmanagedCallConvAttribute`. `CallConv::GetDefaultUnmanagedCallingConvention()` define o padrão por plataforma.

### Obtendo endereços

| Origem | Mecanismo |
|---|---|
| `&Metodo` em C# (`delegate*` gerenciado) | `ldftn` — endereço do método gerenciado (precode ou código) |
| `[UnmanagedCallersOnly]` + `&Metodo` | `ldftn` produz um ponteiro **nativo** invocável de fora; atributo em `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/UnmanagedCallersOnlyAttribute.cs` |
| `NativeLibrary.GetExport`, `Marshal.GetFunctionPointerForDelegate` | endereço nativo / thunk de reverse-P/Invoke (veja `dllimportcallback.cpp`) |
| `delegate*` -> delegate | `Marshal.GetDelegateForFunctionPointer` (delegate com `_extraData == UnmanagedMarker`, ver arquivo 01) |

`COMDelegate::ThrowIfInvalidUnmanagedCallersOnlyUsage` (`comdelegate.cpp`, ~linha 1940) rejeita usos inválidos de `[UnmanagedCallersOnly]` (por exemplo criar delegate para ele).

### Invocação

`calli` com assinatura de function pointer. Para chamadas não gerenciadas dentro de código gerenciado, o JIT embute o *transition frame* (`InlinedCallFrame`, ver `docs/design/coreclr/botr/clr-abi.md`, seções "Per-frame PInvoke initialization" e "Per-call-site PInvoke work"): troca para modo preemptivo (`m_fPreemptiveGCDisabled = 0`), chama, volta, e testa `g_TrapReturningThreads`. `SuppressGCTransition` (atributo em `SuppressGCTransitionAttribute.cs`) elimina essa troca; o doc diz "The below is performed when the GC transition is not suppressed".

`src/coreclr/vm/instancecalli.cpp` implementa suporte a "instance calli" intrínseco (método estático cuja assinatura de declaração vira chamada de instância via `calli`; `_ASSERTE(IsStatic())` confirmado no cabeçalho). O helper gerenciado correspondente é `src/coreclr/System.Private.CoreLib/src/System/Reflection/InstanceCalliHelper.cs`.

### O que function pointers NÃO fazem

- Não carregam alvo (`this`/closure): sem captura.
- Não são rastreados pelo GC (não mantêm o dono do código vivo). Um `delegate*` para método de um assembly colecionável pode ficar pendente (inferência a partir do fato de não haver `LoaderAllocator`, que existe apenas em `Delegate._helperObject`).
- O JIT precisa conhecer a assinatura estática; erro de assinatura = comportamento indefinido (sem verificação em runtime).


### Exemplo ilustrativo (C#, para fixar o modelo)

```csharp
// gerenciado: ldftn + calli (sem alocação)
static int Dobro(int x) => x * 2;
delegate*<int, int> f = &Dobro;
int r = f(21);

// não gerenciado: convenção na assinatura
[UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
static int Callback(int x) => x + 1;
delegate* unmanaged[Cdecl]<int, int> cb = &Callback;
```

O primeiro vira `ldftn Dobro` seguido de `calli int32(int32)`; o segundo vira `ldftn Callback` e `calli unmanaged cdecl int32(int32)`. (A forma de IL é a da ECMA-335/C# 9; o consumo de `UnmanagedCallersOnly` está em `callconvbuilder.cpp`.)

### Onde o JIT trata a assinatura de `calli`

1. O importador (`importer.cpp`, `CEE_CALLI`) lê a assinatura do `StandAloneSig`.
2. O byte de convenção vira `CorInfoCallConv` (`UNMANAGED = 0x9` para nativo).
3. `CallConvBuilder` refina a convenção (base + modificadores) quando há `modopt`.
4. Para chamadas não gerenciadas dentro de código gerenciado, o JIT usa o frame inline de PInvoke (barato) descrito em `clr-abi.md`; se precisa de marshalling, chama um IL stub.
5. Para `SuppressGCTransition`, os passos de troca de modo GC são omitidos.

### Comparação com outros mecanismos de "ponteiro para código"

| Mecanismo | Tipo no runtime | Mantém código vivo? | Carrega estado? |
|---|---|---|---|
| `delegate*` | `FnPtrTypeDesc` | não | não |
| `delegate` | classe derivada de `MulticastDelegate` | sim (via GC) | sim |
| `RuntimeMethodHandle` | struct com handle | não | não |
| `IntPtr` de `GetFunctionPointer()` | `nint` | não | não |

### Armadilhas (derivadas do que o runtime protege)

- Ponteiro para método de assembly colecionável pode ficar inválido após descarregamento.
- `[UnmanagedCallersOnly]` não pode ser chamado diretamente de código gerenciado nem convertido em delegate (verificação em `ThrowIfInvalidUnmanagedCallersOnlyUsage`).
- Exceção não tratada atravessando fronteira nativa não é suportada (regra geral de interop).
- Em x86 a convenção nativa padrão difere (Stdcall vs C), por isso `GetDefaultUnmanagedCallingConvention()` existe e é condicionada por plataforma.

### Decisões que sua linguagem precisa tomar

| Decisão | Opções | Consequência |
|---|---|---|
| Sintaxe | tipo `fn(A)->B` com atributo de convenção | simples; convenção vira parte do tipo |
| Captura | proibir (como `delegate*`) | permite uso em FFI sem thunk |
| Igualdade | por endereço | trivial; thunks podem quebrar identidade |
| Conversão para closure | explícita, via wrapper gerado | evita alocação escondida |
| Verificação | confiança no programador (`unsafe`) | sem custo de runtime |


### Glossário

| Termo | Significado |
|---|---|
| `ldftn` | instrução IL que empilha o endereço de um método |
| `calli` | chamada indireta com assinatura explícita |
| `modopt` | modificador opcional de tipo nos metadados, usado para convenções |
| Reverse P/Invoke | nativo chamando gerenciado |
| Precode | pequeno trecho de código que permite adiar a compilação do método |

### Testes sugeridos

1. Chamar `delegate*` gerenciado e não gerenciado com 0, 1 e 8 argumentos.
2. Passar um `UnmanagedCallersOnly` a uma função C que o chama de volta.
3. Verificar que converter `UnmanagedCallersOnly` em delegate é rejeitado.
4. Medir a diferença de custo contra delegate comum em loop quente.


### Fluxo de vida de um `delegate* unmanaged` exportado

1. O método recebe `[UnmanagedCallersOnly]` e é marcado como ponto de entrada nativo.
2. `&Metodo` produz o endereço (precode ou código) com a convenção pedida.
3. O endereço é entregue ao código nativo.
4. Ao ser chamado, a thread nativa entra em código gerenciado: o runtime precisa mudar para modo cooperativo (reverse P/Invoke, `dllimportcallback.cpp`).
5. Na saída, volta ao modo preemptivo.

## Por quê

- **Zero alocação e zero indireção extra**: um `calli` direto, útil em loops quentes e em runtime/libs de baixo nível.
- **Interop honesto**: código nativo só entende endereços + convenção; o delegate gerenciado precisa de thunk (reverse-PInvoke), o ponteiro direto não.
- **Convenção na assinatura** permite ao JIT gerar o prólogo/epílogo certos sem metadados externos.
- `[UnmanagedCallersOnly]` permite que um método gerenciado seja chamado de fora sem marshalling por IL stub e sem delegate (ponto de entrada direto, inclusive em NativeAOT).

## Trade-offs

| Aspecto | Delegate | Function pointer |
|---|---|---|
| Captura de estado | sim | não |
| Alocação | objeto | nenhuma |
| Segurança de tipo em runtime | binding verificado | confiança no programador |
| Multicast | sim | não |
| Interop nativo | thunk + marshalling | direto (se blittable) |
| Sobrevivência a descarregar assembly | gerenciada | não gerenciada |

- Exige `unsafe`; sem verificabilidade.
- Genéricos: o tipo `delegate*<T,...>` não pode ser argumento genérico em C# (restrição da linguagem, conhecimento geral).
- Reflection com function pointers é limitada (tipos de ponteiro de função aparecem como `IntPtr` em muitas APIs; `ModifiedType.CoreCLR.cs` em `System/Reflection` trata modopt/modreq).

## Como aplicar na sua linguagem

1. Ofereça dois tipos distintos: **closure** (par env+code, GC-aware) e **ponteiro de função** (só endereço). Não tente unificá-los.
2. Faça a convenção de chamada parte do **tipo** do ponteiro, não do valor. Use a mesma representação que o .NET: base + lista de modificadores (Cdecl, Stdcall, SuppressGCTransition, MemberFunction).
3. Defina um atributo equivalente a `UnmanagedCallersOnly` para exportar funções da linguagem para C sem thunk de delegate, e proíba usá-lo como delegate.
4. Defina a política de transição GC (se houver GC): `SuppressGCTransition` só é seguro para chamadas curtas, sem callbacks e sem bloqueio — documente isso como contrato.
5. Para plataformas sem JIT (AOT), garanta que `&f` seja resolvido em tempo de link; o `ldftn` do .NET pode devolver um precode que atrasa a compilação.
6. Se a linguagem tiver coleta de código (descarregamento), decida quem mantém o código vivo: ponteiros crus normalmente não mantêm.

## Referências

- `src/coreclr/vm/typedesc.h` (`FnPtrTypeDesc`)
- `src/coreclr/inc/corinfo.h` (`CorInfoCallConv`, linha 651)
- `src/coreclr/vm/callconvbuilder.hpp`, `callconvbuilder.cpp`
- `src/coreclr/vm/instancecalli.cpp`, `src/coreclr/System.Private.CoreLib/src/System/Reflection/InstanceCalliHelper.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/UnmanagedCallersOnlyAttribute.cs`, `UnmanagedCallConvAttribute.cs`, `SuppressGCTransitionAttribute.cs`
- `src/coreclr/vm/comdelegate.cpp` (`ThrowIfInvalidUnmanagedCallersOnlyUsage`)
- `docs/design/coreclr/botr/clr-abi.md` (PInvoke frame)
- `docs/design/features/source-generator-pinvokes.md` (IL stub invoca o export via `calli`)
