# P/Invoke e marshalling de dados

## Mecanismo no .NET

P/Invoke é o mecanismo para um método gerenciado chamar uma função nativa exportada por biblioteca. O runtime transforma cada declaração `[DllImport]` em uma chamada de **IL stub** gerado sob demanda, que faz marshalling, a transição de modo GC e a chamada `calli`.

### Fluxo (conforme `docs/design/features/source-generator-pinvokes.md`)

Para o exemplo `[DllImport("Kernel32.dll")] static extern bool QueryPerformanceCounter(out long c);`:

1. Na invocação, o método é identificado como externo e que exige marshalling; com base em `DllImportAttribute` e na assinatura é gerado o IL stub.
2. O runtime localiza a biblioteca (influenciado por `AssemblyLoadContext` e `NativeLibrary`).
3. Procura o export pelo nome (ou `EntryPoint`); respeita convenção ANSI/Unicode no Windows, tentando sufixo `A`/`W`.
4. O IL stub é chamado como qualquer método .NET.
5. O stub faz marshalling dos argumentos e invoca o export via `calli`.
6. No retorno, limpa e faz marshalling dos valores de saída.

### Onde está no código

| Peça | Arquivo |
|---|---|
| Geração do stub, resolução de `NDirect`, flags | `src/coreclr/vm/dllimport.cpp` (6994 linhas), `dllimport.h` |
| Marshalers IL por tipo | `src/coreclr/vm/ilmarshalers.cpp` (5030 linhas), `ilmarshalers.h` |
| Emissor de IL do stub | `src/coreclr/vm/stubgen.cpp`, `stubgen.h` |
| Cache de stubs | `src/coreclr/vm/ilstubcache.cpp`, `ilstubcache.h` |
| Reverse P/Invoke (nativo -> gerenciado), thunks | `src/coreclr/vm/dllimportcallback.cpp` |
| Convenção de chamada | `src/coreclr/vm/callconvbuilder.cpp` |
| COM interop (CCW/RCW) | `comcallablewrapper.cpp`, `runtimecallablewrapper.cpp` |

`ilmarshalers.h` define uma hierarquia `ILMarshaler` com classes confirmadas, entre outras: `ILBlittableValueClassMarshaler`, `ILPointerMarshaler`, `ILDelegateMarshaler`, `ILBoolMarshaler` (variantes `ILWinBoolMarshaler`, `ILCBoolMarshaler`, `ILVtBoolMarshaler`), `ILCUTF8Marshaler`, `ILWSTRMarshaler`, `ILCSTRMarshaler`, `ILBSTRMarshaler`, `ILSafeHandleMarshaler`, `ILCriticalHandleMarshaler`, `ILValueClassMarshaler`, `ILObjectMarshaler`, `ILInterfaceMarshaler`, `ILLayoutClassPtrMarshaler`, `ILFixedWSTRMarshaler`, `ILFixedCSTRMarshaler`, `ILOptimizedAllocMarshaler` (buffers com alocação otimizada na pilha) e outros.

### Blittable vs não blittable

Tipos *blittable* (mesma representação gerenciada e nativa: inteiros, floats, ponteiros, structs só de blittable) são copiados sem conversão (`ILCopyMarshalerBase`/`ILBlittableValueClassMarshaler`). Os demais exigem conversão: `string` -> `char*`/`wchar_t*`/`BSTR`, `bool` -> 1/4 bytes, delegates -> function pointer thunk, objetos -> pinning ou cópia.

### Transição de modo GC

Em `docs/design/coreclr/botr/clr-abi.md` (seções "Per-frame PInvoke initialization" e "Per-call-site PInvoke work"):

- O JIT embute uma `InlinedCallFrame` e, a cada chamada: seta `m_pDatum` (MethodDesc), ativa o frame com `m_pCallerReturnAddress`, passa a modo **preemptivo** (`m_fPreemptiveGCDisabled = 0`), chama, volta a cooperativo, testa `g_TrapReturningThreads` (se ligado, chama `CORINFO_HELP_STOP_FOR_GC`).
- "no GC pointers may be live in registers" durante a chamada: o LSRA do RyuJIT adiciona `RefTypeKillGCRefs`.
- Por isso refs devem ser *pinned* e passados como ponteiros nativos; o doc diz que argumentos/retorno são seguros porque não podem ser GC refs.
- `SuppressGCTransition` pula essa troca (para chamadas curtíssimas).

### LibraryImport: marshalling por source generator

O mesmo doc `source-generator-pinvokes.md` explica a motivação: o marshalling interno exige atualização do runtime para corrigir bugs, tipos novos exigem mudanças no runtime, `ICustomMarshaler` tem "penalidade substancial de desempenho", depuração do IL stub é difícil, e o AOT/linker não enxerga código gerado em runtime. A solução: `[LibraryImport]` com gerador em `src/libraries/System.Runtime.InteropServices/gen/LibraryImportGenerator/` que emite C# (blittable `DllImport` + código de marshalling) em tempo de compilação. Metas declaradas: "Allow P/Invoke interop evolution independently of runtime" e "No reflection at runtime, compatible in an AOT scenario". Não-metas: "100% parity" com o marshalling antigo.

Para ReadyToRun: `docs/design/features/readytorun-pinvoke.md`.

### Delegates em interop

`COMDelegate::ValidateDelegatePInvoke`, `GetILStubMethodDesc`, `GetStubForILStub` (`comdelegate.cpp`) geram stubs de marshalling para delegates passados como ponteiros de função. `ILDelegateMarshaler` converte delegate <-> function pointer.


### Exemplo de IL stub (pseudocódigo ilustrativo)

Para `static extern bool QueryPerformanceCounter(out long c);` o stub gerado faz, conceitualmente:

```text
stub(ref long c) -> bool
  // setup: blittable 'out long' usa o endereço direto (pin)
  // transição GC: feita pelo JIT ao compilar o calli do stub
  r = calli unmanaged int32 (long*)  [endereço resolvido do export]
  // converte retorno int32 -> bool (ILWinBoolMarshaler: != 0)
  // cleanup (libera buffers, SafeHandle etc.)
  return converted
```

Modelo mental baseado em `source-generator-pinvokes.md` (passos 4-6) e na hierarquia `ILMarshaler` de `ilmarshalers.h`. Os nomes exatos das fases internas do marshaler não foram verificados.

### Escolha do marshaler por tipo (derivado das classes existentes)

| Tipo gerenciado | Marshaler (em `ilmarshalers.h`) |
|---|---|
| `int`, `long`, `float`, ponteiros | `ILCopyMarshalerBase` / `ILPointerMarshaler` |
| struct só de blittable | `ILBlittableValueClassMarshaler` |
| `bool` | `ILBoolMarshaler` com variantes `ILWinBoolMarshaler`, `ILCBoolMarshaler`, `ILVtBoolMarshaler` |
| `string` | `ILCUTF8Marshaler`, `ILCSTRMarshaler`, `ILWSTRMarshaler`, `ILBSTRMarshaler` |
| `StringBuilder` | `ILWSTRBufferMarshaler`, `ILCSTRBufferMarshaler`, `ILUTF8BufferMarshaler` (base `ILOptimizedAllocMarshaler`) |
| `SafeHandle` / `CriticalHandle` | `ILSafeHandleMarshaler`, `ILCriticalHandleMarshaler` |
| delegate | `ILDelegateMarshaler` |
| classe com layout | `ILLayoutClassPtrMarshaler`, `ILBlittablePtrMarshaler` |
| texto fixo em struct | `ILFixedWSTRMarshaler`, `ILFixedCSTRMarshaler` |
| interface COM | `ILInterfaceMarshaler` |

### Resolução da biblioteca (resumo do doc)

1. Nome da DLL do atributo mais hooks `AssemblyLoadContext` e `NativeLibrary`.
2. Export pelo nome do método ou por `EntryPoint`.
3. No Windows, tentativa do sufixo `A`/`W`.
4. `docs/design/features/dllmap.md` trata mapeamento de nomes de biblioteca.

### Erros e `SetLastError`

`DllImport.SetLastError` instrui o stub a capturar o último erro do SO logo após a chamada, antes que o runtime o sobrescreva (comportamento da API; a implementação está no código do stub em `dllimport.cpp`/`ilmarshalers.cpp`, não lida em detalhe aqui).

### Reverse P/Invoke

Para passar um delegate a C, o runtime gera um thunk nativo (`dllimportcallback.cpp`; `UMEntryThunkData` aparece em `clr-abi.md`) que entra em modo cooperativo, recupera o delegate, faz marshalling inverso e invoca. `UnmanagedCallersOnly` elimina o delegate e o thunk intermediário (arquivo 02).

### Matriz de decisão: qual caminho usar

| Situação | Caminho recomendado |
|---|---|
| Só blittable, chamada curta | `DllImport` blittable ou `LibraryImport` sem marshalling |
| Strings e bool | `LibraryImport` (gerador) com `StringMarshalling` |
| Estruturas complexas, COM | marshalling embutido ou `ComWrappers` |
| Callback de C | `UnmanagedCallersOnly` + `delegate*` |
| AOT/trimming | gerador em compile-time |


### Glossário

| Termo | Significado |
|---|---|
| Blittable | tipo com mesma representação gerenciada e nativa |
| IL stub | método sintético gerado pelo runtime |
| Pinning | fixar objeto para que o GC não o mova |
| Modo preemptivo | a thread declara que não toca em objetos gerenciados |
| CCW/RCW | wrappers de COM para exposição e consumo |

## Por quê

- **Esconder ABI nativo do programador**: o stub ajusta layout, codificação e ownership.
- **Transição de GC é obrigatória**: a thread nativa pode bloquear por muito tempo; o GC precisa conseguir pausá-la sem esperar. Modo preemptivo declara "não toco em objetos gerenciados".
- **Stubs gerados e cacheados** (`ilstubcache`) evitam reconstruir marshalling por chamada; cada combinação assinatura+flags tem um stub compartilhado.
- **Source generation** retira o marshalling de dentro do runtime: versionamento independente, AOT/trimming amigáveis.

## Trade-offs

- IL stub em runtime: flexível, mas bloqueia AOT total, é difícil de depurar e cresce o runtime (6994 + 5030 linhas só aqui).
- Marshalling implícito (strings, bool) tem custos escondidos (alocação, cópia) e semânticas surpreendentes (tamanho de `bool`).
- Pinning em chamadas longas fragmenta o heap; copiar custa CPU.
- `SuppressGCTransition` torna GC não-responsivo se a chamada demorar.
- Compatibilidade: bugs de marshalling viram comportamento esperado (`source-generator-pinvokes.md` cita isto como razão para não corrigir no runtime).
- Source generator: o usuário vê código gerado, mas não tem paridade total de comportamento.

## Como aplicar na sua linguagem

1. Defina a **classe blittable** na linguagem: tipos cuja representação é idêntica à do C. Faça com que chamada de FFI com apenas blittable seja uma chamada direta, sem stub.
2. Para tipos não blittable, prefira **geração em tempo de compilação** (macro/generator) a stubs em runtime; separe o marshaller em biblioteca (política) do mecanismo de chamada (primitivo).
3. Especifique ownership: quem aloca e quem libera cada buffer (`ILOptimizedAllocMarshaler` usa pilha para buffers pequenos; é bom padrão).
4. Se tiver GC, modele a **transição**: estados cooperativo/preemptivo, e uma regra de "sem refs vivas em registradores" na chamada.
5. Ofereça handles seguros (equivalente a `SafeHandle`) para recursos nativos, impedindo vazamento e uso-após-liberação durante a chamada.
6. Defina resolução de biblioteca com hook (equivalente a `NativeLibrary`/`AssemblyLoadContext`), e mapeamento de nome (`EntryPoint`).
7. Planeje reverse-FFI (callbacks de C) desde cedo: precisa de entrada nativa com transição GC invertida (`dllimportcallback.cpp`).

## Referências

- `docs/design/features/source-generator-pinvokes.md`
- `docs/design/coreclr/botr/clr-abi.md` (PInvoke frames)
- `docs/design/features/readytorun-pinvoke.md`
- `src/coreclr/vm/dllimport.cpp`, `dllimport.h`, `dllimportcallback.cpp`
- `src/coreclr/vm/ilmarshalers.cpp`, `ilmarshalers.h`
- `src/coreclr/vm/stubgen.cpp`, `ilstubcache.cpp`
- `src/coreclr/vm/comcallablewrapper.cpp`, `runtimecallablewrapper.cpp`
- `src/libraries/System.Runtime.InteropServices/gen/LibraryImportGenerator/`
- `docs/design/features/COM-activation.md`, `IJW-activation.md`, `dllmap.md`
