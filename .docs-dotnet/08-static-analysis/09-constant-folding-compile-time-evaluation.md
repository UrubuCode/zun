# Const folding e avaliação em tempo de compilação

> Escopo: o `const` e a dobra de constantes do **compilador C#** vivem no Roslyn (fora deste repo). O que o `dotnet/runtime` mostra, e este documento cobre, é a avaliação de constantes em **quatro outros níveis**: (1) o JIT (`gtFoldExpr*` e *value numbering*), (2) o ILLink (propagação de constantes entre métodos + remoção de ramos), (3) substituições declarativas (`ILLink.Substitutions.xml`, feature switches) e (4) o interpretador de *static constructors* do NativeAOT (`TypePreinit`).

## Mecanismo no .NET

### Nível 1 — JIT: `gtFoldExpr`

`src/coreclr/jit/gentree.cpp`, função `Compiler::gtFoldExpr(GenTree* tree)` (~linha 15605). Pontos confirmados:

- Pré-condições: `assert(!optValnumCSE_phase)` — "nunca somos chamados do CSE. Várias otimizações abaixo assumem que é seguro trocar operandos se um é constante".
- `if (!opts.Tier0OptimizationEnabled()) return tree;` — **em Tier0 (minopts) não há folding**.
- Despacha por aridade: folhas (nada), binário (`gtFoldExprBinary`), unário (`gtFoldExprUnary`), condicional (`gtFoldExprConditional`) e, se `FEATURE_HW_INTRINSICS`, `gtFoldExprHWIntrinsic`.
- Existem variantes por tipo de constante: `gtFoldExprUnaryConstInt/Lng/Dbl`, `gtFoldExprBinaryConst*` (declaradas em `compiler.h` ~linhas 4120-4131).
- Aritmética com flag de overflow só dobra se não overflowar; caso contrário vira helper (arquivo 08, `gtFoldExprForOverflow`).
- O resultado é escrito "in place" via `gtBashTreeToConstInt` (o nó é transformado em constante, preservando `FieldSeq`).

Exemplo de caso (linhas ~18372): para `GT_ADD` com dois `int` constantes, o JIT usa `static_cast<uint32_t>` para somar com semântica de wrap e depois cria a constante.

### Nível 1b — JIT: value numbering

`valuenum.h/.cpp` implementa *value numbering*: `VNForIntCon`, `EvalFuncForConstantArgs(var_types, VNFunc, vn0[, vn1])`, `EvalFuncForConstantFPArgs`, `EvalOp<T>(vnf, v0[, v1])`. Se todos os argumentos de uma função são constantes (VN de constante), o resultado também é uma constante VN. Essa avaliação se aplica **a expressões em SSA através de blocos**, e alimenta `assertionprop` e `rangecheck` (arquivo 08). A diferença para `gtFoldExpr`: `gtFoldExpr` reescreve a árvore; VN apenas *conhece* o valor e o assertion prop depois substitui.

### Nível 2 — ILLink: propagação de constantes entre métodos + remoção de ramos

Documento de design: `docs/design/tools/illink/constant-propagation-and-branch-removal.md` (199 linhas). Implementação: `src/tools/illink/src/linker/Linker.Steps/UnreachableBlocksOptimizer.cs`.

Objetivo declarado: "ILLink implementa otimização que propaga valores constantes entre métodos e, com base neles, determina ramos inalcançáveis e os remove. Isso significa que o código no ramo removido não é varrido por dependências, que por sua vez não são marcadas e podem ser removidas".

Exemplo do documento:

```csharp
public bool Is32Bit { get => false; }                  // retorna constante
public int SizeOfIntPtr { get { if (Is32Bit) return 4; else return 8; } }  // logo, sempre 8
public void CopyMemory() { if (Is32Bit) CopyUsingDWords(); else CopyUsingQWords(); } // 1º ramo removido
```

**Não-objetivo explícito**: "por ora o ILLink não fará inlining de chamadas". Deixar as chamadas facilita depuração (breakpoint no callee sempre é atingido).

**Algoritmo (descrito no documento)** — notável porque resolve um problema global com visão local:

- Estruturas: dicionário `método → valor` (estados: *Processed but not changed*, *Processed and is not constant*, ou uma instrução com o valor constante de retorno); uma **pilha de processamento** (lista encadeada, para mover nós para o topo) e um índice método→nó.
- Loop: olha o topo; faz scan do corpo, detecta chamados úteis; se algum chamado não foi processado, o empilha (ou move para o topo, se já está na pilha) e desiste do método atual (volta ao loop); se o scan terminou, remove ramos inalcançáveis, analisa se o método retorna constante e armazena o resultado.
- **Detecção de ciclo por versionamento**: um número de versão global é incrementado a cada push/pop; cada nó guarda `LastAttemptVersion`. Se um nó vai ser processado de novo sem que a versão mudou, há ciclo.
- **Resolução de ciclo**: força-se o processamento de um método tratando dependências não processadas como "processadas, não constantes". Antes disso, uma etapa adicional puxa para o topo nós fora do ciclo (ex.: um `Helper` constante) para não perder otimizações — importante porque, se o `Helper` depender de um *feature switch*, deixá-lo não resolvido geraria avisos de código que deveria ser removido.
- Motivo para o desenho local: "carregamento preguiçoso de assemblies" — antes e durante a marcação, não há garantia de que todos os assemblies foram descobertos; mas a otimização precisa terminar antes de o `MarkStep` processar o método (para que dependências de ramos mortos não sejam marcadas).

No código, `UnreachableBlocksOptimizer.cs` tem as classes aninhadas `BodyReducer` (reduz o corpo e remove instruções inalcançáveis via `RemoveUnreachableInstructions(BitArray reachable)`), `CallInliner` (`ApplyTemporaryInlining` — inlining **temporário** só para a análise), `ConstantExpressionMethodAnalyzer` e `BodySweeper`.

### Nível 3 — Substituições declarativas e feature switches

`docs/workflow/trimming/ILLink-files.md` e `feature-switches.md` descrevem `ILLink.Substitutions.xml`: "direciona a ferramenta a substituir o corpo de um método por um `throw` ou por retorno de constante". Sintaxe verificada em `feature-switches.md`:

```xml
<method signature="System.Boolean get_EnableUnsafeUTF7Encoding()" body="stub" value="false"
        feature="System.Text.Encoding.EnableUnsafeUTF7Encoding" featurevalue="false" />
```

Também há substituições por arquitetura em `src/libraries/System.Private.CoreLib/src/ILLink/`: `ILLink.Substitutions.32bit.xml`, `64bit.xml`, `LittleEndian.xml`, `NoX86Intrinsics.xml`, `NoArmIntrinsics.xml`, `NoWasmIntrinsics.xml`, `iOS.xml`, `Shared.xml` — que fixam como constantes, por exemplo, `System.IntPtr.get_Size()` (`value="8"` em `64bit.xml`) e `Aes.get_IsSupported()`/`Vector256.get_IsHardwareAccelerated()` (`value="false"` em `NoX86Intrinsics.xml`). Efeito: o Nível 2 faz o resto (propagação + ramos mortos).

Atributos que ligam propriedades a *switches* sem XML: `FeatureSwitchDefinitionAttribute(string switchName)` e `FeatureGuardAttribute(Type featureType)` (CoreLib, `System/Diagnostics/CodeAnalysis/`). No NativeAOT, `ILCompiler.Compiler/Compiler/SubstitutionProvider.cs` os lê (ver arquivo 06) e `SubstitutedILProvider.cs` aplica as substituições ao IL antes da compilação; `BodySubstitution.cs`/`BodySubstitutionParser.cs` lêem o XML.

Os padrões de guarda aceitos (para que a constante seja reconhecida) estão em `docs/design/tools/illink/feature-checks.md`: acesso direto, negação `!`, comparação com `true/false`, etc.

### Nível 4 — NativeAOT: `TypePreinit`

`src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/TypePreinit.cs`: "Classe que calcula o estado inicial de campos estáticos de um tipo **interpretando o static constructor**". Valores são representados por instâncias de uma classe abstrata `Value`; valores não-referência são arrays de bytes; há tratamento cuidadoso de semântica de atribuição para value types. Relacionados: `PreinitializationManager.cs` (decide se o cctor é *lazy*, eager ou já pré-inicializado) e as opções do compilador (`ILCompilerRootCommand.cs`): `--preinitstatics` ("Interpret static constructors at compile time if possible (implied by -O)") e `--nopreinitstatics`. Em outras palavras: **execução parcial do código de inicialização em tempo de compilação**, gravando o resultado como dados da imagem.

## Por quê

- **Código menor e mais rápido** sem mudar semântica: constantes conhecidas eliminam ramos, e ramos eliminados eliminam dependências (efeito em cascata no trimmer).
- **Habilitar configuração por feature**: `IsSupported` constante é a ponte entre "o que o usuário configurou" e "o que o binário contém".
- **Simplificar o resto do pipeline**: dobrar cedo reduz o IR que o restante analisa.
- **Startup**: pré-inicializar estáticos evita executar cctors na inicialização do app.
- **Preservar depuração (ILLink)**: não fazer inlining para manter chamadas visíveis.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| JIT: sem folding em Tier0 | Compilação rápida | Código Tier0 menos otimizado |
| Fold in-place | Eficiente | Cuidado: outros passes checam nós originais ("we intentionally do not destroy the nodes", comentário em `gtFoldExprUnary`) |
| Overflow checado não é dobrado quando overflowa | Preserva exceção | Casos especiais e `gtFoldExprForOverflow` só em `fgGlobalMorph` |
| ILLink propaga constantes sem inlining | Depuração previsível | Menos precisão que inlining completo |
| ILLink: algoritmo local com pilha e detecção de ciclo por versão | Funciona com assemblies carregados preguiçosamente | Complexidade e heurísticas ("complex loop cases") |
| Substituições XML | Controle explícito, sem alterar código-fonte | Duplicação de conhecimento (código + XML); fácil de ficar fora de sincronia |
| `TypePreinit` interpreta cctors | Startup e tamanho | Interpretador limitado: só aceita subconjunto de IL; falha → cctor roda em runtime |

## Como aplicar na sua linguagem

1. **Tenha pelo menos três "camadas" de constantes**: (a) literais e `const` na linguagem, (b) propriedades booleanas "de capacidade" que o toolchain conhece como constantes (`IsSupported`), (c) estado de inicialização estática avaliável em compile-time.
2. **Dobre com a semântica exata do tempo de execução**: wrap/trap, arredondamento de float, shifts. O JIT usa aritmética sem sinal e depois `static_cast` — modelo para evitar UB na sua implementação host.
3. **Dobra que pode lançar exceção deve preservá-la**: se a operação provadamente overflowa, mantenha a operação (ou vire chamada ao helper), não substitua por constante.
4. **Propague constantes interprocedurais de forma barata**: somente funções "puras o suficiente", retorno único/constante, sem inlining. Use uma pilha de trabalho + detecção de ciclo por versão (simples e robusta).
5. **Faça a remoção de ramos antes da análise de alcançabilidade** (como o ILLink antes de `MarkStep`): é o que torna `if (!Feature.IsSupported) return;` eliminar dependências.
6. **Defina um mecanismo declarativo de override** (equivalente a `ILLink.Substitutions.xml`) para plataformas e features, e fixe uma sintaxe estável.
7. **Considere um interpretador de inicializadores estáticos** (`TypePreinit`) com subconjunto bem definido e fallback seguro (se não conseguir, executa em runtime).
8. **Marque constantes obrigatórias** (`ConstantExpected`) para APIs cuja semântica exige valor conhecido, e verifique no front-end.
9. **Documente o que NÃO é dobrado** (ex.: chamadas com efeitos colaterais) para evitar surpresas.

## Apêndice: visão consolidada e exemplo em cascata

### Quatro níveis lado a lado

| Nível | Representação | Quem avalia | Escopo | Modifica o quê |
|---|---|---|---|---|
| JIT `gtFoldExpr` | Árvore `GenTree` | Compilador JIT | Expressão/bloco | Reescreve o nó para constante |
| JIT value numbering | VNs em SSA | JIT | Método inteiro | Não reescreve; informa assertion prop/range |
| ILLink `UnreachableBlocksOptimizer` | IL (Mono.Cecil) | Trimmer | Entre métodos (global, lazy) | Remove instruções/branches |
| NativeAOT `TypePreinit` | IL de static ctors | Compilador AOT | Tipo | Gera dados de imagem para estáticos |

### Exemplo em cascata (combinando feature switch + ILLink + JIT/AOT)

```csharp
static class Util {
    public static bool IsAotMode => !RuntimeFeature.IsDynamicCodeSupported;   // constante no AOT
    public static void Fazer() {
        if (IsAotMode) CaminhoEstatico();
        else           CaminhoDinamico();      // usa Reflection.Emit
    }
}
```

1. Nativo: `IsDynamicCodeSupported` é `false` (`RuntimeFeature.NativeAot.cs`) e também um *feature switch* (`FeatureSwitchDefinition`).
2. O compilador trata a leitura da propriedade como constante (`SubstitutionProvider`).
3. A propagação transforma `IsAotMode` em `true` (método que retorna constante), e `Fazer` tem o branch `else` removido (`UnreachableBlocksOptimizer`/`SubstitutedILProvider`).
4. `CaminhoDinamico` nunca é marcado, portanto é removido (e seus avisos IL3050 desaparecem).
5. No JIT (se não AOT), `gtFoldExpr`/`fgFoldConditional` fazem o mesmo para constantes que o JIT consegue ver (por exemplo `Vector256.IsHardwareAccelerated` e `IsSupported` de intrínsecos são tratados como constantes pelo JIT — informação geral; os arquivos de substituição do ILLink mostram a mesma ideia para o trimmer).

### O que o ILLink escolheu **não** fazer (e por quê)

- Não faz inlining geral (depuração previsível).
- Não promete resolver todos os ciclos otimamente: o documento descreve "casos complexos de loop" e uma heurística de duas etapas.
- Não propaga constantes de campos ou de estado mutável (o doc descreve apenas retorno constante de métodos e substituições).

### Lista de verificação para implementar um propagador de constantes interprocedural leve

1. Defina o que é "método candidato": corpo pequeno, sem efeitos colaterais relevantes, retorno único.
2. Mantenha um estado por método em {não analisado, não constante, constante(v)}.
3. Use uma pilha de trabalho e um contador de versão para detectar ciclos.
4. Ao quebrar ciclos, trate pendências como "não constante" (conservador) — mas antes processe dependências fora do ciclo.
5. Rode a remoção de ramos **antes** de qualquer análise de alcançabilidade.
6. Mantenha IL/bytecode verificável após a remoção (arquivo 11).
7. Exponha um mecanismo declarativo de sobrescrita (XML/atributos).

## Referências

- `src/coreclr/jit/gentree.cpp` (`gtFoldExpr`, `gtFoldExprUnary/Binary`, `gtFoldExprConst*`, `gtBashTreeToConstInt`, `gtFoldExprForOverflow`)
- `src/coreclr/jit/compiler.h` (declarações `gtFoldExpr*`)
- `src/coreclr/jit/valuenum.h`, `valuenum.cpp` (`EvalFuncForConstantArgs`, `EvalOp`, `VNForIntCon`)
- `docs/design/tools/illink/constant-propagation-and-branch-removal.md`
- `src/tools/illink/src/linker/Linker.Steps/UnreachableBlocksOptimizer.cs`, `BodySubstituterStep.cs`, `BodySubstitutionParser.cs`
- `docs/workflow/trimming/ILLink-files.md`, `feature-switches.md`; `docs/design/tools/illink/feature-checks.md`
- `src/libraries/System.Private.CoreLib/src/ILLink/ILLink.Substitutions.*.xml`
- `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/SubstitutionProvider.cs`, `SubstitutedILProvider.cs`, `TypePreinit.cs`, `PreinitializationManager.cs`
- `src/coreclr/tools/aot/ILCompiler/ILCompilerRootCommand.cs` (`--preinitstatics`)
