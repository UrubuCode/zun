# Intrinsics reconhecidos pelo JIT

## Mecanismo no .NET

Um *intrinsic* é um método da biblioteca que o compilador JIT reconhece e substitui por IR/instrução especial em vez de compilar/chamar o corpo gerenciado. Há duas famílias: **named intrinsics** (identificadas por nome de classe/método) e **hardware intrinsics** (vetores e ISA).

### Marcação no código-fonte

O atributo `[Intrinsic]` (`System.Runtime.CompilerServices.IntrinsicAttribute`) é colocado nos métodos. Exemplo confirmado em `src/libraries/System.Private.CoreLib/src/System/Math.cs` (~linha 123):

```csharp
[Intrinsic]
[MethodImpl(MethodImplOptions.AggressiveInlining)]
public static double Abs(double value)
{
    const ulong mask = 0x7FFFFFFFFFFFFFFF;
    ulong raw = BitConverter.DoubleToUInt64Bits(value);
    return BitConverter.UInt64BitsToDouble(raw & mask);
}
```

O corpo é uma **implementação de fallback** completa. O JIT pode ignorá-la e emitir uma instrução (`andps` etc.), mas se não puder expandir, o corpo gerenciado é executado. O VM expõe o bit ao JIT em `CORINFO_FLG_INTRINSIC = 0x00400000` ("This method MAY have an intrinsic ID") e `CORINFO_FLG_INTRINSIC_TYPE = 0x200` (tipo marcado com `[Intrinsic]`) em `src/coreclr/inc/corinfo.h` (linhas ~750-762). Há também um "quick check" `isIntrinsic` equivalente a `getMethodAttribs(ftn) & CORINFO_FLG_INTRINSIC`.

### Tabela de intrinsics nomeados

`src/coreclr/jit/namedintrinsiclist.h` declara `enum NamedIntrinsic : unsigned short` com centenas de entradas, por exemplo:

- `NI_System_Math_Abs`, `Acos`, `Sqrt`, `FusedMultiplyAdd`, `NI_System_Half_op_Addition` (operadores como intrinsics, ver arquivo 06);
- `NI_System_Buffer_Memmove`, `NI_System_SpanHelpers_Memmove`;
- `NI_System_Type_get_IsValueType`, `NI_System_Type_op_Equality`, `NI_System_Type_GetTypeFromHandle`, `NI_System_Activator_CreateInstance_T`;
- `NI_System_Runtime_CompilerServices_RuntimeHelpers_IsKnownConstant`, `..._IsReferenceOrContainsReferences`, `..._CreateSpan`;
- `NI_System_Threading_Interlocked_CompareExchange`, `Exchange`, `ExchangeAdd`, `And`, `Or`;
- `NI_System_ArgumentNullException_ThrowIfNull`, `NI_System_Enum_HasFlag`.

O comentário do arquivo avisa: ao adicionar um intrinsic que usa `GT_INTRINSIC` e pode lançar, atualizar `OperMayThrow` e `fgValueNumberAddExceptionSet`.

### Caminho no JIT

1. `lookupNamedIntrinsic(method)` mapeia o `MethodHandle` ao `NamedIntrinsic` (usado em `importercalls.cpp`, ex. linha ~1559).
2. `impIntrinsic(...)` (`importercalls.cpp`, ~linha 3330) tenta expandir. Calcula `mustExpand`: `gtIsRecursiveCall(method, false) && !(methodFlags & CORINFO_FLG_VIRTUAL)` (~linha 3354), isto é, um intrinsic cujo corpo chama a si mesmo **precisa** ser expandido (senão recursão infinita).
3. Para hardware intrinsics: `impHWIntrinsic(ni, ...)` (~linha 3533). Se `mustExpand` e não há suporte, emite `CORINFO_HELP_THROW_NOT_IMPLEMENTED` (`impUnsupportedNamedIntrinsic`).
4. Fora do tier de otimização o JIT frequentemente **não** expande (linha ~3661: "we're fine if intrinsic decides to not expand itself in this case unlike mustExpand"; ~3796: `!mustExpand && opts.OptimizationDisabled()`).
5. Hardware intrinsics têm tabelas por arquitetura: `hwintrinsiclistxarch.h`, `hwintrinsiclistarm64.h`, `hwintrinsiclistarm64sve.h`, `hwintrinsiclistwasm.h`; importação em `hwintrinsic.cpp`, `hwintrinsicxarch.cpp`, `hwintrinsicarm64.cpp`; codegen em `hwintrinsiccodegenxarch.cpp` etc.

### Hardware intrinsics: documentação de design

`docs/design/features/hw-intrinsics.md` ("Implementation of Hardware Intrinsics in CoreCLR") divide em: tipos vetoriais agnósticos de plataforma, "JIT Implementation" (Platform Target Information, Importation, Hardware Intrinsics Table, IR, Lowering, Register Allocation), Code Generation, Encoding, Testing. `docs/design/features/arm64-intrinsics.md` cobre ARM64.

### Regras de determinismo e AOT

`docs/design/coreclr/botr/vectors-and-intrinsics.md` define regras importantes:

- "At a direct call site, intrinsic expansion is optional. If the importer cannot generate an implementation, it returns `nullptr` and leaves a managed call."
- "Indirect invocation through reflection, a delegate, or a function pointer executes the intrinsic method body. That body must select the same result as direct call-site expansion in the same process."
- APIs *não determinísticas* entre hardwares (ex. `ConvertToIntegerNative`, `*Estimate`) só garantem determinismo **dentro de um processo**.
- ReadyToRun registra pré-requisitos de ISA positivos/negativos (`notifyInstructionSetUsage(isa, supportEnabled, preserveNegativeDependency)`), e `[CompExactlyDependsOn(typeof(Avx2))]` marca dependências em código de CoreLib.

### Outros intrinsics "especiais"

`StubHelpers.NextCallReturnAddress` é um intrinsic que representa o endereço de retorno da próxima chamada, usado pelo mecanismo de tail call por helpers (`tailcalls-with-helpers.md`); `fgCanFastTailCall` o trata ("Uses NextCallReturnAddress intrinsic" desativa fast tail call).


### Três exemplos de expansão (comportamento conceitual)

1. `Math.Abs(double)` com `[Intrinsic]`: o JIT pode emitir uma única instrução de máscara em vez de chamar o método; sem expansão (Tier0, interpretador), o corpo C# acima roda.
2. `typeof(T).IsValueType` (`NI_System_Type_get_IsValueType`): após instanciação genérica, T é conhecido e o JIT pode reduzir a constante, eliminando ramos mortos; vale também para `NI_System_Type_op_Equality`.
3. `RuntimeHelpers.IsReferenceOrContainsReferences<T>()` (`NI_System_Runtime_CompilerServices_RuntimeHelpers_IsReferenceOrContainsReferences`): constante em tempo de JIT, usado para escolher entre limpeza rápida e segura para GC.

A transformação exata depende do tier e da arquitetura; os nomes dos intrinsics estão confirmados em `namedintrinsiclist.h`.

### Ciclo de vida de um intrinsic

```text
[Intrinsic] no método C#            (CoreLib)
   -> VM seta CORINFO_FLG_INTRINSIC (corinfo.h)
   -> JIT: lookupNamedIntrinsic(method) -> enum NamedIntrinsic
   -> impIntrinsic / impHWIntrinsic:
        expandiu?  sim -> nó especial (GT_INTRINSIC, GT_HWINTRINSIC, constante ...)
                   não -> chamada normal ao corpo gerenciado (inlining possível)
   -> morph / value numbering / lowering / LSRA / codegen
```

`namedintrinsiclist.h` alerta que um `GT_INTRINSIC` que pode lançar exige atualizar `OperMayThrow` e `fgValueNumberAddExceptionSet`; intrinsics participam da análise de efeitos do JIT.

### Hardware intrinsics: classes de tipos

`hw-intrinsics.md` descreve tipos vetoriais agnósticos de plataforma e classes por ISA. O `vectors-and-intrinsics.md` mostra `[CompExactlyDependsOn(typeof(Avx2))]` e `[CompExactlyDependsOn(typeof(PackedSimd))]`, confirmando suporte a WASM SIMD. A propriedade `IsSupported` de cada classe é tratada como constante pelo JIT, o que permite escrever caminhos com fallback por ISA sem custo em runtime (descrição geral do padrão; ver o doc para as regras exatas).

Para CoreLib, o método que usa uma ISA deve ser anotado com `CompExactlyDependsOn` ou `CompOpportunisticallyDependsOn`, para o crossgen2 registrar as ISAs assumidas.

### Checklist para criar um intrinsic (derivado dos avisos do repositório)

- Criar o método com corpo correto e `[Intrinsic]`.
- Adicionar entrada no enum `NamedIntrinsic` e no mapeamento de nomes (`lookupNamedIntrinsic`).
- Implementar a expansão em `impIntrinsic` (ou `impHWIntrinsic`).
- Tratar `mustExpand` (recursão) e o modo sem otimização.
- Atualizar `OperMayThrow` e a numeração de valores se puder lançar.
- Garantir igualdade com o fallback (NaN, saturação) ou classificar como não determinístico.
- Para ReadyToRun: registrar pré-requisitos de ISA.

### Tipos de efeito a considerar no design

| Categoria | Exemplo | Risco |
|---|---|---|
| Puro / aritmético | `Math.Sqrt` | divergência numérica |
| Memória | `Buffer.Memmove` | aliasing, barreiras de escrita |
| Atômico | `Interlocked.*` | semântica de memória (`docs/design/specs/Memory-model.md`) |
| Constante de tipo | `Type.IsValueType` | só vale após especialização |
| Controle | `ThrowIfNull` | afeta layout de blocos frios |


### Glossário

| Termo | Significado |
|---|---|
| Named intrinsic | método reconhecido pelo nome de classe/método |
| Hardware intrinsic | operação de ISA exposta como API (SIMD, bit ops) |
| `mustExpand` | o JIT deve expandir, sem alternativa de chamada |
| Fallback | corpo gerenciado usado quando não há expansão |
| ISA | conjunto de instruções (SSE, AVX2, AdvSimd, SVE, PackedSimd) |

### Testes sugeridos

1. Executar cada API com intrinsic ligado e desligado (modo sem otimização) e comparar resultados.
2. Invocar a mesma API por delegate e por reflexão e comparar com a chamada direta.
3. Para APIs "native"/"estimate", verificar apenas determinismo dentro de um processo.

## Por quê

- **Desempenho previsível sem sintaxe nova**: bibliotecas expressam `Math.Sqrt`, `Interlocked.Add`, `Vector128.Add` como chamadas normais; o JIT sabe o que fazer.
- **Fallback correto**: o corpo gerenciado mantém semântica quando não há expansão (interpretador, Tier0, plataformas sem suporte, reflection).
- **Evolução independente**: novos intrinsics não exigem sintaxe nem opcode IL.
- **Constantes e tipos como fatos do compilador**: `IsKnownConstant`, `typeof(T).IsValueType` viram constantes após especialização genérica.

## Trade-offs

- Acoplamento biblioteca-JIT: renomear um método quebra silenciosamente a otimização (sem erro de compilação, só perda de desempenho).
- Lista longa de enums e tabelas por arquitetura é custo de manutenção.
- Fallback precisa ser idêntico ao intrinsic — bugs sutis em NaN, overflow, arredondamento. O doc exige igualdade no mesmo processo.
- Em AOT, o código usa ISA conservador ou grava dependências para revalidar em runtime.
- Usuários de terceiros não podem criar intrinsics: só o CoreLib tem esse poder.

## Como aplicar na sua linguagem

1. Escolha um **mecanismo declarativo** (atributo/anotação) para marcar funções de biblioteca como intrínsecas, sempre com corpo de fallback.
2. Separe: (a) intrinsics **de semântica** (alocação, atômicos, memmove), (b) intrinsics **de hardware** (SIMD), (c) intrinsics **de constantes** (tipo/tamanho/`IsKnownConstant`).
3. Defina a política `mustExpand`: quando não há fallback possível (ex. instrução exclusiva), erro de compilação ou throw definido.
4. Garanta equivalência de resultados entre expandido e fallback e especifique quais APIs podem divergir por hardware (rotule como "native"/"estimate").
5. Se houver AOT, guarde no artefato o conjunto de capacidades de ISA assumido e revalide no carregamento.
6. Testes: um teste que roda cada intrinsic com e sem expansão (tier0, interpreter) e compara.

## Referências

- `src/coreclr/jit/namedintrinsiclist.h`
- `src/coreclr/jit/importercalls.cpp` (`impIntrinsic`, `lookupNamedIntrinsic`)
- `src/coreclr/jit/hwintrinsic.cpp`, `hwintrinsiclistxarch.h`, `hwintrinsiclistarm64.h`, `hwintrinsiclistwasm.h`
- `src/coreclr/inc/corinfo.h` (`CORINFO_FLG_INTRINSIC`)
- `src/libraries/System.Private.CoreLib/src/System/Math.cs`
- `docs/design/features/hw-intrinsics.md`, `arm64-intrinsics.md`
- `docs/design/coreclr/botr/vectors-and-intrinsics.md`
