# Dead code / código inalcançável

> Escopo: "dead code" tem três sentidos diferentes e o .NET trata cada um em uma camada. (a) *código inalcançável* (nenhum caminho de controle chega a ele); (b) *código morto por valor* (calculado e nunca usado — "dead stores"); (c) *código não referenciado* (alcançável do ponto de vista local, mas nenhum chamador o alcança a partir das raízes — é o trimming). O aviso de linguagem C# "unreachable code detected" (CS0162) é do Roslyn e não está neste repositório; aqui descrevo apenas o que se confirma nos fontes do runtime.

## Mecanismo no .NET

### 1. JIT — remoção de blocos inalcançáveis

`src/coreclr/jit/fgopt.cpp`, função template `Compiler::fgRemoveUnreachableBlocks(CanRemoveBlockBody canRemoveBlock)` (~linha 79). Comentário do cabeçalho (confirmado):

- "Alguns blocos (marcados com `BBF_DONT_REMOVE`) não podem ser removidos mesmo inalcançáveis; nesse caso são convertidos em blocos `throw`. Blocos auxiliares internos de throw e o único bloco de retorno (se houver) nunca são considerados inalcançáveis."
- O parâmetro `canRemoveBlock` é um predicado: nas fases **iniciais** depende dos *conjuntos de alcançabilidade* (reachability sets, que precisam estar válidos); na fase **final** depende de uma busca em profundidade (DFS) a partir de `fgFirstBB`: "tudo o que não é visitado é inalcançável (com exceção de blocos de handler/filter)".
- "A fase de remoção de blocos inalcançáveis acontece **duas vezes**."
- Primeiro marca blocos com `BBF_REMOVED` (varrendo `Blocks()`), depois os remove com `fgRemoveBlock(block, unreachable)` (`compiler.h` ~linha 6912). A fase `fgDfsBlocksAndRemove` (`compiler.h` ~7015) combina DFS e remoção.

Observe que a alcançabilidade em CFG com **exceções** exige exceções à regra (handlers são alcançáveis por fluxo excepcional, não por arestas normais).

### 2. JIT — dobrar condicionais gera código morto

`morph.cpp`, `Compiler::fgFoldConditional(BasicBlock* block)` (~linha 13171): "tenta dobrar condicionais e otimizar blocos `BBJ_COND` ou `BBJ_SWITCH`". Pontos confirmados:

- `if (opts.OptimizationDisabled()) return FOLD_DID_NOTHING;` — comentário: "Não queremos tornar nenhum código inalcançável". Em Tier0/minopts não se dobra, justamente para não produzir código morto para tratar.
- Se a última instrução é uma chamada (`GT_CALL`) — throw incondicional — o bloco vira `BBJ_THROW` por `fgConvertBBToThrowBB(block)`.
- Se a condição (`gtEffectiveVal`) é constante, o salto condicional é substituído por incondicional e o ramo não escolhido perde a aresta; então `fgRemoveUnreachableBlocks` o limpa.
- Resultados são do tipo `FoldResult` (`FOLD_DID_NOTHING`, `FOLD_CHANGED_CONTROL_FLOW`, …).

Ou seja: **constant folding (arquivo 09) → ramo inalcançável → remoção**. É o mesmo encadeamento que o ILLink reproduz sobre IL.

### 3. JIT — ramos redundantes por implicação

`redundantbranchopts.cpp`, `Compiler::optRedundantBranches()` percorre a árvore de dominância (`DomTreeVisitor`) e elimina condicionais cujo resultado já é implicado por uma condição dominante (usa asserções/value numbers — mesma infraestrutura do arquivo 08). Faz parte do mesmo tema: um ramo que nunca é tomado é código morto.

### 4. JIT — stores mortos e locais não usados

`liveness.cpp` tem `fgTryRemoveDeadStoreLIR` ("tenta remover uma store morta em LIR", ~linha 2635) e a família `fgComputeLife*` (`fgComputeLifeTrackedLocalUse`, `fgComputeLifeTrackedLocalDef`, `fgComputeLifeUntrackedLocal`). Se um local é definido e nunca lido antes de nova definição/saída, a store é removida (respeitando efeitos colaterais: `GTF_EXCEPT` e `GTF_SIDE_EFFECT` em `gentree.h` impedem remover nós que podem lançar ou chamar). Também existe `optRemoveRedundantZeroInits` (`compiler.h` ~linha 7817), que remove inicializações redundantes com zero (relacionado à discussão de `.locals init` no arquivo 03).

### 5. ILLink — remoção de ramos inalcançáveis sobre IL

`src/tools/illink/src/linker/Linker.Steps/UnreachableBlocksOptimizer.cs` (arquivo 09 descreve a propagação de constantes). Aqui interessa a parte de **reescrever IL válido** ao cortar código. Em `BodyReducer.RemoveUnreachableInstructions(BitArray reachable)` (~linha 981), o comentário explica um problema sutil, que é uma lição para qualquer implementador de remoção de código morto sobre bytecode:

> Se pretendemos remover a última instrução, nós a substituímos por `ret` (não `nop`), mas não podemos eliminá-la completamente porque pode acontecer de a última instrução mantida ser um **branch condicional** — nesse caso, para manter o IL válido, tem que haver algo depois do branch condicional (o ramo `else`). Então, nesse caso, injeta-se `ldnull; throw;` no fim — esse ramo nunca deve ser alcançável e é sempre válido (`ret` pode precisar retornar um valor do tipo certo, o que é complicado de construir do nada).

Essa necessidade vem diretamente do **verificador de IL** (arquivo 01): `VerifierError.MethodFallthrough` ("cair do fim do método sem retornar") é emitido em `ILImporter.Verify.cs` linha ~2867. O commit recente mais próximo na história do repo, "Fix ILVerify crash when a method ends with a conditional branch or switch", mostra que o assunto é real (aparece nos commits recentes do repositório ao início desta sessão).

Estrutura do arquivo: `BodyReducer` (reduz o corpo), `CallInliner` (inlining temporário só para análise), `BodySweeper` (varre e reconstrói), `ConstantExpressionMethodAnalyzer`. `SweepStep.cs` remove o que o `MarkStep` não marcou, e `CodeRewriterStep` aplica reescritas.

### 6. ILLink e NativeAOT — código **não referenciado**

Dead code no sentido (c): `MarkStep.cs` (arquivo 04) é a alcançabilidade; `SweepStep` apaga o resto. Com branches removidos antes de marcar, as dependências do ramo morto **nunca são marcadas** ("code in the removed branch is not scanned for its dependencies" — `constant-propagation-and-branch-removal.md`). No NativeAOT, o grafo de dependências (`ILCompiler.DependencyAnalysisFramework`, `UsageBasedMetadataManager`) só compila e emite o que é alcançado; `SubstitutedILProvider.cs` aplica substituições antes.

Mecanismo de controle pelo desenvolvedor para o caso (c): `ILLink.Descriptors.xml` (forçar preservação), `DynamicDependency`, `RootAssembly`, `TrimMode`. Em `docs/workflow/trimming/ILLink-files.md`, o repo diz que descritores devem ser evitados e justificados: "como usar um descritor significa que o código será preservado sempre, mesmo no aplicativo final".

### 7. Validação de efeitos na verificação

`ILImporter.Verify.cs` verifica estruturas de controle como `MethodFallthrough`, `BadBranch`, `BadJumpTarget`, `MethodEnd`; um verificador que também detectasse código inalcançável precisaria de análise de alcançabilidade. Nos fontes consultados não identifiquei um erro de "unreachable code" no enum `VerifierError` — o verificador valida estrutura, não elimina código morto (afirmo isso apenas com base na lista de erros em `VerifierError.cs`).

## Por quê

- **Menor tamanho e menor superfície de dependência** (ILLink/NativeAOT): cada ramo morto removido evita arrastar tipos e métodos.
- **Mais velocidade** (JIT): menos blocos, melhor alocação de registradores e layout.
- **Corretude**: remover um ramo é seguro apenas se a condição é *provadamente* constante; por isso a dobra respeita efeitos (`GTF_EXCEPT`) e overflow.
- **Pré-requisito para outras análises**: a análise de alcançabilidade e o *constant propagation* precisam de CFG limpo; o JIT roda a remoção em duas fases, uma antes e outra depois de dobrar.
- **Feature switches**: ramos mortos por configuração não devem gerar avisos de trimming (`feature-checks.md`).

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Não dobrar condicionais em minopts | Compilação rápida e IR simples | Código morto permanece no Tier0 |
| Remoção em duas fases (reachability sets / DFS) | Primeira fase barata, segunda precisa | Complexidade de manter reachability sets válidos |
| Blocos `DONT_REMOVE` viram `throw` em vez de sumirem | Estrutura de EH e retorno preservada | Blocos "zumbi" no CFG |
| ILLink remove ramos sobre IL e precisa manter IL válido | Menos dependências marcadas | Casos de borda (`ldnull; throw`) para satisfazer o verificador |
| Sem inlining no ILLink (só temporário) | Depuração previsível | Menos oportunidades de dobra |
| Descritores XML para forçar preservação | Escape para reflexão/nativo | Preserva código mesmo se inútil; difícil de auditar |

## Como aplicar na sua linguagem

1. **Separe os três conceitos** em sua documentação e mensagens: inalcançável (aviso de linguagem), morto por valor (otimização), não referenciado (tree-shaking). Cada um tem análise e política diferentes.
2. **Defina inalcançabilidade no CFG com DFS a partir da entrada**, tratando handlers de exceção como raízes adicionais. Mantenha uma lista de blocos "intocáveis" (retorno único, helpers de throw) como o JIT faz.
3. **Dobre condicionais constantes antes da remoção**, mas somente quando otimização está habilitada; em modo de depuração, preserve o código para breakpoints.
4. **Quando remover código sobre bytecode, mantenha o bytecode verificável**: não deixe branch condicional sem fall-through válido; use um terminador impossível (`ldnull; throw` no .NET) quando o tipo de retorno complica um `ret`.
5. **Marque efeitos** (pode lançar, chama função) em cada nó e só remova stores/ramos quando sem efeito.
6. **Permita declarar "funcionalidades desligadas"** (feature switches) como constantes que alimentam a remoção, para que caminhos de código opcionais saiam do binário.
7. **Emita aviso de código inalcançável no front-end** (não está no runtime, é do compilador C#) para ajudar o programador, e *não* o trate como erro.
8. **Teste a remoção** com casos mínimos que verificam o IL de saída (ILLink: `Mono.Linker.Tests.Cases` com expectativas de "kept"/"removed"; JIT: `JITDUMP` e testes `FileCheck`).

## Apêndice: exemplo de CFG e glossário

### Exemplo: do `if` constante ao bloco removido

```csharp
static void F() {
    if (Config.Habilitado)     // Config.Habilitado vira constante false (substituição / feature switch)
        A();
    else
        B();
    C();
}
```

CFG antes: `BB0 [cond] -> BB1(A) | BB2(B)`, `BB1 -> BB3`, `BB2 -> BB3`, `BB3(C) -> ret`.

1. Dobra: `fgFoldConditional` (JIT, só com otimização) ou propagação do ILLink troca o salto condicional por incondicional para `BB2`.
2. Alcançabilidade: `BB1` perde sua única aresta de entrada; `fgRemoveUnreachableBlocks` marca `BBF_REMOVED` e o remove (a menos que tenha `BBF_DONT_REMOVE`, caso em que vira `throw`).
3. Consequência para o ILLink: `A()` nunca é marcado; se `A` só era chamado ali, é removido pelo `SweepStep`.
4. Consequência para o NativeAOT: `A` nem entra no grafo de dependências.

### Glossário

| Termo | Significado |
|---|---|
| Bloco inalcançável | Sem caminho do bloco de entrada (e de raízes de EH) até ele |
| `BBF_DONT_REMOVE` | Flag de bloco que não pode ser removido (vira `throw`) |
| `BBJ_THROW` | Bloco que termina em throw |
| Store morta | Atribuição a variável nunca lida depois |
| `lvMustInit` | Variável que precisa de inicialização no prólogo |
| `FoldResult` | Resultado de uma tentativa de dobra: nada, mudou fluxo, etc. |
| Tree shaking | Remoção de código não alcançável a partir das raízes |
| Raiz | Ponto de partida da alcançabilidade (entry point, descritor, `DynamicDependency`) |

### Hierarquia de custo/benefício (quando remover código morto)

1. **Front-end**: aviso ao programador (cedo, sem custo de runtime).
2. **IR/bytecode**: remoção segura em tempo de compilação, mantendo verificabilidade.
3. **Link/AOT**: remoção de não referenciados — maior ganho em tamanho.
4. **JIT**: remoção por constantes de plataforma (`IsSupported`, `IntPtr.Size`) em tempo de execução — ganho em velocidade e tamanho de código nativo.

Cada nível usa informações disponíveis apenas ali: o front-end não sabe a plataforma final; o JIT conhece o hardware real; o linker conhece o conjunto completo de assemblies.

### Perguntas de projeto

1. O seu compilador avisa sobre código inalcançável ou o remove silenciosamente?
2. Em modo debug, as linhas removidas ainda aceitam breakpoints?
3. Quem é raiz de alcançabilidade além do ponto de entrada (exports, reflexão, interop)?

### Armadilhas a evitar

- Remover código com efeitos observáveis (exceções, escrita volátil): o JIT usa `GTF_EXCEPT` e `GTF_SIDE_EFFECT` para proteger.
- Esquecer handlers de exceção como raízes de alcançabilidade: o JIT exclui explicitamente handler/filter do critério de "não visitado".
- Deixar o IL inválido após cortar (ver o caso do branch condicional final no ILLink).
- Quebrar a depuração: não otimizar em `OptimizationDisabled()` (comentário em `fgFoldConditional`).
- Tratar código gerado por geradores/compilador como "morto" porque o usuário não o escreveu (o trimmer precisa do grafo de código gerado: `CompilerGeneratedState`).

## Referências

- `src/coreclr/jit/fgopt.cpp` (`fgRemoveUnreachableBlocks`), `compiler.h` (`fgRemoveBlock`, `fgDfsBlocksAndRemove`, `optRemoveRedundantZeroInits`)
- `src/coreclr/jit/morph.cpp` (`fgFoldConditional`)
- `src/coreclr/jit/redundantbranchopts.cpp` (`optRedundantBranches`)
- `src/coreclr/jit/liveness.cpp` (`fgTryRemoveDeadStoreLIR`, `fgComputeLife*`)
- `src/tools/illink/src/linker/Linker.Steps/UnreachableBlocksOptimizer.cs`, `SweepStep.cs`, `MarkStep.cs`
- `src/coreclr/tools/ILVerification/ILImporter.Verify.cs` (`MethodFallthrough`), `VerifierError.cs`
- `docs/design/tools/illink/constant-propagation-and-branch-removal.md`, `feature-checks.md`
- `docs/workflow/trimming/ILLink-files.md`
- `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/SubstitutedILProvider.cs`, `UsageBasedMetadataManager.cs`
