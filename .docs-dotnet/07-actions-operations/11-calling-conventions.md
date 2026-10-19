# Calling conventions (convenções de chamada)

## Mecanismo no .NET

"Convenção de chamada" no .NET existe em três níveis: a **convenção gerenciada** do próprio CLR (ABI interno, por arquitetura), as **convenções nativas** (cdecl, stdcall, thiscall, fastcall, mais as do SO) usadas em interop, e *extensões* como async e varargs. O documento canônico é `docs/design/coreclr/botr/clr-abi.md` (992 linhas).

### Metadados: byte de convenção na assinatura

`src/coreclr/inc/corinfo.h` (~linha 651) define `CorInfoCallConv`:

| Valor | Constante | Significado |
|---|---|---|
| 0x0 | `CORINFO_CALLCONV_DEFAULT` | gerenciada |
| 0x5 | `CORINFO_CALLCONV_VARARG` | varargs gerenciado |
| 0x6 | `FIELD` | assinatura de campo |
| 0x7 | `LOCAL_SIG` | assinatura de locais |
| 0x8 | `PROPERTY` | propriedade |
| 0x9 | `CORINFO_CALLCONV_UNMANAGED` | não gerenciada (convenção refinada por modopt) |
| 0xb | `NATIVEVARARG` | "used ONLY for IL stub PInvoke vararg calls" |
| 0x10 | `GENERIC` | método genérico |
| 0x20 | `HASTHIS` | tem `this` implícito |
| 0x40 | `EXPLICITTHIS` | `this` explícito |
| 0x80 | `PARAMTYPE` | parâmetro de tipo oculto, "Passed last" |
| 0x100 | `ASYNCCALL` | convenção async |

As convenções C/STDCALL/THISCALL/FASTCALL (0x1-0x4) aparecem **comentadas** no enum: no CLR moderno elas são escolhidas por `modopt` (`CallConvCdecl`, `CallConvStdcall`, `CallConvThiscall`, `CallConvFastcall`, `CallConvMemberFunction`, `CallConvSuppressGCTransition`) na assinatura de `calli` ou pelo atributo `UnmanagedCallConvAttribute` / `DllImport.CallingConvention`.

`CallConvBuilder` (`src/coreclr/vm/callconvbuilder.hpp`) agrega `CorInfoCallConvExtension CallConvBase` e `CallConvModifiers`; `CallConv::TryGetCallingConventionFromUnmanagedCallConv` lê o atributo; `TryGetCallingConventionFromUnmanagedCallersOnly(MethodDesc*, ...)` lê `UnmanagedCallersOnly`. `GetDefaultUnmanagedCallingConvention()` retorna o padrão por plataforma (ver código).

### ABI gerenciado: o que o clr-abi.md documenta

Seções reais do documento (ordem do arquivo):

- Windows/Non-Windows ABI: o CLR reusa o ABI da plataforma (Windows x64, System V, ARM/ARM64) com ajustes.
- **Frame pointer chains**.
- **The `this` pointer**: passado como primeiro argumento (RCX em Windows x64, RDI em System V — `clr-abi.md` linha ~584).
- **Varargs**.
- **Generics**: parâmetro de contexto genérico oculto (seção "Hidden parameters"); a ordem em relação a `this`/return buffer é definida no doc.
- **Async**: "Async calling convention is additive to other calling conventions when supported... adds an extra `Continuation` parameter and an extra return, which semantically takes precedence when not `null`." Limitado a chamadas estáticas/virtuais comuns; sem PInvokes ou varargs (seções "Returning `Continuation`", "Passing `Continuation` argument").
- **By-value value types passed by reference** (structs grandes passados por ponteiro a cópia).
- **Return buffers**: retorno de structs grandes por buffer do chamador.
- **Small primitive returns/arguments**: regras de extensão de sinal para tipos pequenos.
- **Per-frame / per-call-site PInvoke work**: frames de transição (arquivo 04).
- **Funclets** (EH), **Cloned finallys**, etc.
- **Interpreter ABI** e ABI WebAssembly/portable entrypoints (~linhas 702+, 888): "Direct managed R2R-to-R2R calls still use the portable-entrypoint calling convention so that the same call site can target either native R2R code or an interpreter thunk."

### Detalhes de struct

`docs/design/coreclr/jit/struct-abi.md` e `first-class-structs.md` descrevem como o JIT classifica structs (registro, pilha, multi-reg `multi-reg-call-nodes.md`), e `jit-call-morphing.md` descreve como argumentos são preparados (`fgMorphArgs`).

### Conversões entre convenções

- **Delegate estático**: *shuffle thunk* desloca argumentos (arquivo 01).
- **Delegate aberto de instância com retorno por buffer**: `ThisPtrRetBufPrecode` troca `this` e return buffer (`method-descriptor.md`).
- **Reverse P/Invoke**: thunk recupera `UMEntryThunkData` (`clr-abi.md` linha ~165: "Native-to-managed method and delegate stubs pass the `UMEntryThunkData`").
- **IL stubs** de P/Invoke traduzem convenção gerenciada -> nativa.


### Ordem dos argumentos no ABI portátil (WASM): exemplo documentado

O `clr-abi.md` (~linha 867) especifica:

```text
$sp, [this], [retbuf], [generic context], [async continuation], [user args...], pep_ptr
```

Serve de modelo concreto de "argumentos ocultos em ordem fixa". O mesmo documento diz que `int F(int x)` tem assinatura Wasm `(func (param i32 i32 i32) (result i32))`: `$sp`, `x` e `pep_ptr`. É a convenção da variante WebAssembly; as demais arquiteturas têm regras próprias no mesmo arquivo.

### Mapa de regras por tema (onde olhar)

| Tema | Seção do `clr-abi.md` |
|---|---|
| `this` em registrador | "The `this` pointer" |
| Varargs | "Varargs" |
| Contexto genérico | "Generics", "Hidden parameters" |
| Async/continuation | "Async" |
| Struct por referência | "By-value value types passed by reference" |
| Retorno grande | "Return buffers" |
| Tipos pequenos | "Small primitive returns", "Small primitive arguments" |
| Transição nativa | "Per-frame PInvoke initialization", "Per-call-site PInvoke work" |
| Exceções | "Funclets", "Funclet parameters", "Registers on entry to a funclet" |
| Registradores salvos | "Fixed set of callee-saved registers" |
| Interpretador | "Interpreter ABI details" |

### Convenções nativas relevantes para interop

| Modificador | Uso típico |
|---|---|
| `CallConvCdecl` | C em Unix e Windows |
| `CallConvStdcall` | Win32 API em x86 |
| `CallConvThiscall` | métodos C++ em x86 |
| `CallConvFastcall` | raro |
| `CallConvMemberFunction` | métodos de instância C++ |
| `CallConvSuppressGCTransition` | otimização, não é convenção em si |

Os nomes `CallConv*` são da API pública `System.Runtime.CompilerServices`; o repositório os consome em `callconvbuilder.cpp`. Em x64 há essencialmente uma convenção por SO; em x86 as diferenças são reais.

### Por que o CLR lista tantos parâmetros ocultos

Cada recurso que altera a forma de chamar acrescenta um parâmetro implícito: genéricos compartilhados (contexto), retorno grande (retbuf), async (continuation), WASM/interpretador (`pep_ptr`), IL stubs (`UMEntryThunkData` ou parâmetro secreto). Stubs, tail calls (12), reflexão (08) e delegates (01) precisam conhecer todos.

### Conversão de convenção no código do repositório

- Shuffle thunk de delegate estático: `comdelegate.cpp` (`GenerateShuffleArray`).
- Precodes: `method-descriptor.md`.
- IL stubs: `ilstubcache.cpp`, `stubgen.cpp`.
- `callstubgenerator.cpp`/`.h`: gerador de stubs de chamada (uso exato não confirmado aqui).

### Passagem de structs: o que decidir

| Decisão | Opções vistas no .NET |
|---|---|
| Struct pequeno | em registradores (classificação SysV, multi-reg) |
| Struct grande | por referência a cópia (Windows x64) ou na pilha |
| Retorno | em registradores ou via return buffer |
| HFA/RISC-V | regras específicas (seção "RISC-V only") |

Detalhes por arquitetura estão em `struct-abi.md`.

### Checklist de teste de ABI

1. Funções com 0, 1, 6, 7+ argumentos inteiros e flutuantes.
2. Structs de tamanhos 1, 2, 4, 8, 12, 16, 24, 32 bytes, homogêneos e mistos.
3. Retorno de cada struct acima.
4. Varargs com flutuante.
5. Chamada cruzada: gerenciado -> nativo -> gerenciado.

## Por quê

- **Reuso do ABI da plataforma** permite chamar código nativo e ser chamado por ele sem tradução na maioria dos casos, e permite depuradores/profilers padrão.
- **Argumentos ocultos explícitos** (generics, async, retbuf) são exigidos pelo modelo do runtime (genéricos compartilhados, continuations).
- **Convenção nos metadados** deixa o JIT e o stub generator escolherem sem heurística.
- **Convenção por modificadores** é aditiva: `SuppressGCTransition` e `MemberFunction` combinam com base.

## Trade-offs

- Um ABI próprio quebra interop; reusar o da plataforma herda suas peculiaridades (x86 stdcall, Apple ARM64, SysV structs).
- Argumentos ocultos complicam todos os call sites e thunks (cada nova adição — como async — multiplica casos em stubs, tail calls e reflexão).
- Portar para uma nova arquitetura exige reimplementar centenas de linhas de regras (RISC-V, LoongArch, Wasm são seções separadas).
- Varargs, structs e returns pequenos variam por plataforma; fonte de bugs.
- Convenções "aditivas" (async) são difíceis de combinar com PInvoke/varargs — excluídos explicitamente.

## Como aplicar na sua linguagem

1. Escolha **uma convenção interna estável** e documente-a como o `clr-abi.md` faz (this, retbuf, contexto genérico, ordem). Prefira alinhar com o ABI nativo da plataforma, desviando só onde necessário.
2. Represente convenção como **parte do tipo da função**, base + modificadores, não como flag solta.
3. Reserve espaço para argumentos ocultos (contexto de closure, tipo genérico, continuation) e fixe a **ordem** desde o início.
4. Regule a passagem de structs: tamanho limite para registros, passagem por cópia oculta, retbuf.
5. Separe convenção de FFI (explícita, suportada por atributo) da interna (não exposta).
6. Prove interoperabilidade com testes que chamam funções nativas com cada convenção suportada.
7. Se a linguagem for AOT, decida se existe um "entrypoint portátil" (como `pep_ptr` do CLR no WASM) para tornar chamadas indiretas uniformes entre interpretador e código nativo.

## Referências

- `docs/design/coreclr/botr/clr-abi.md`
- `src/coreclr/inc/corinfo.h` (`CorInfoCallConv`)
- `src/coreclr/vm/callconvbuilder.hpp`, `callconvbuilder.cpp`
- `docs/design/coreclr/jit/struct-abi.md`, `first-class-structs.md`, `multi-reg-call-nodes.md`, `jit-call-morphing.md`
- `docs/design/coreclr/botr/method-descriptor.md`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/UnmanagedCallConvAttribute.cs`, `UnmanagedCallersOnlyAttribute.cs`
- `docs/design/coreclr/botr/guide-for-porting.md`
