# Inlining e otimizações do RyuJIT

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.
> Números de heurística (100 bytes, profundidade 20 etc.) são os `#define`/defaults lidos em
> `src/coreclr/jit/` hoje e podem mudar.

## Mecanismo no .NET

### Inlining: quando e como

O inlining acontece no front end, durante o **morph** (fase `Morph - Inlining`, função
`fgInline`; arquivo `src/coreclr/jit/fginline.cpp`). Segundo `docs/design/coreclr/jit/ryujit-overview.md`
("Inlining"):

1. Para cada call site, uma **máquina de estados percorre a IL do candidato** e estima o tamanho
   do código nativo correspondente, usando um conjunto de heurísticas, incluindo o tamanho
   estimado do método atual, para decidir se vale.
2. Se vale, cria-se **um objeto `Compiler` separado** e roda-se a fase de *importation* para
   gerar a árvore do método inlinado.
3. O inlining pode ser **abortado** a qualquer momento se aparecer algo que o torne
   não-lucrativo ou incorreto.
4. Se bem-sucedido, as árvores do inlinee são incorporadas ao "pai" (inliner), com argumentos
   e valores de retorno devidamente transformados.

Como nota o plano `docs/design/coreclr/jit/inlining-plans.md`, um inliner tem quatro aspectos:
**machinery** (transformar a IR), **legality** (regras ditadas pelo runtime), **ability** (se a
machinery consegue e as fases seguintes aguentam) e **profitability** (heurística de
conveniência).

### Observações e vetos

`src/coreclr/jit/inline.def` lista todas as "observações" que o JIT faz sobre callee e
call site, com um impacto (`FATAL`, `PERFORMANCE`, `INFORMATION`...). Exemplos reais:

- **Fatais** (veto): `IS_NOINLINE`, `IS_VM_NOINLINE`, `IS_SYNCHRONIZED`, `HAS_NO_BODY`,
  `HAS_MANAGED_VARARGS`, `TOO_MANY_ARGUMENTS`, `TOO_MANY_LOCALS`, `MAXSTACK_TOO_BIG`,
  `LOCALLOC_TOO_LARGE`, `EXPLICIT_TAIL_PREFIX`, `STACK_CRAWL_MARK`, `IS_ARRAY_METHOD`,
  `HAS_UNMANAGED_CALLCONV`... (O arquivo também lista `HAS_EH`, mas há uma configuração
  `JitInlineMethodsWithEH` com padrão 1 em `jitconfigvalues.h:133`; como as duas se
  relacionam não foi investigado.)
- **Performance**: `TOO_MANY_BASIC_BLOCKS`, `TOO_MUCH_IL`, `LDFLD_STATIC_VALUECLASS`.
- **Informação** (aumentam a vontade de inlinar): `ARG_FEEDS_CONSTANT_TEST`, `ARG_FEEDS_TEST`,
  `ARG_FEEDS_CAST`, `ARG_FEEDS_RANGE_CHECK`, `ARG_FEEDS_ISCONST`, `CONST_ARG_FEEDS_ISCONST`,
  `ARG_STRUCT`, `RETURNS_STRUCT`, `ARG_STRUCT_FIELD_ACCESS`, `BINARY_EXRP_WITH_CNS`.

A ideia: um argumento constante que alimenta um teste, um cast ou uma checagem de limites é
sinal de que o inline destravará otimizações no caller.

### Limites configuráveis

Em `src/coreclr/jit/compiler.h:12666-12678`:

| Constante | Valor | Significado |
|---|---|---|
| `DEFAULT_MAX_INLINE_SIZE` | 100 | métodos com mais de 100 **bytes de IL** nunca são inlinados (sobrescrito por `DOTNET_JitInlineSize`) |
| `DEFAULT_MAX_INLINE_DEPTH` | 20 | profundidade máxima de inlining |
| `DEFAULT_INLINE_BUDGET` | 22 | "aumento máximo estimado do tempo de compilação" via inlining (`JitInlineBudget`) |
| `DEFAULT_MAX_FORCE_INLINE_DEPTH` | 1 | profundidade máxima para inlining forçado |
| `ALWAYS_INLINE_SIZE` (`inline.h:1154`) | 16 | callee até esse tamanho de IL vira candidato "sempre" |
| `MAX_BASIC_BLOCKS` (`inlinepolicy.h:156`) | 5 | acima disso, rejeita se não for inline forçado |
| `MAX_INL_ARGS` / `MAX_INL_LCLS` (`inline.h:79-80`) | 32 / 32 | limites de argumentos/locais |
| `DEFAULT_MAX_LOCALLOC_TO_LOCAL_SIZE` | 32 | `localloc` fixo desse tamanho ou menor vira buffer local |

"Inline forçado" (`m_IsForceInline` em `inlinepolicy.cpp`) corresponde, **[inferido]**, ao atributo `AggressiveInlining`. O orçamento
(`inline budget`) limita o *total* de crescimento de tempo de compilação no método.

### Políticas

`src/coreclr/jit/inlinepolicy.h` define uma hierarquia: `LegalPolicy` -> `DefaultPolicy` ->
`ExtendedDefaultPolicy`; mais `DiscretionaryPolicy`, `ModelPolicy`, `ProfilePolicy`,
`RandomPolicy`, `FullPolicy`, `SizePolicy`, `ReplayPolicy`. O nome `ProfilePolicy` sugere uso de dados de PGO **[inferido]**; não li a implementação.
Existe infraestrutura para despejar dados de decisão (`JitInlineDumpData`, `JitInlineDumpXml`)
para treinar/validar modelos - ver `docs/design/coreclr/jit/inlining-plans.md`, que também discute
medir "qualidade do inlining" em três eixos: tempo de geração de código (TP), tempo de execução
(CQ) e tamanho do código.

### Devirtualização guardada (GDV)

Doc `GuardedDevirtualization.md`: quando o JIT não consegue provar o tipo exato do receptor
de uma chamada virtual/interface, pode gerar `if (tipo == T_provável) chamada_direta_inlinável
else chamada_virtual_original`. O tipo provável vem de **perfil** (PGO). O doc original
(proposta para .NET Core 3.0) justifica pelas estatísticas da época: só ~15% das chamadas
virtuais e ~5% das de interface eram devirtualizáveis por análise estática. A fase
`Resolve GDV Checks` (`compphases.h`) e o `importer*.cpp` fazem parte da implementação.
**[confirmado]** que existe; os percentuais acima são do documento de 2019, não medidos hoje.

### O pipeline de otimizações (depois do inlining)

De `ryujit-overview.md` e `compphases.h`:

| Otimização | O que faz (descrição do doc) |
|---|---|
| **Struct promotion / Physical promotion** | Divide structs em locais separados (campos), permitindo enregistrar |
| **Forward substitution** | Elimina locais SDSU substituindo o valor no uso |
| **Local/Global Morph** | Simplificações locais, normalização obrigatória |
| **Find Loops** | Acha loops naturais e os põe em forma canônica (pré-header, saídas só com predecessores do loop) |
| **Invert Loops** | Converte para forma `do..while` |
| **Clone Loops** | Cria versão extra do loop com condições para otimizar mais (ex.: remover bounds checks, guard checks de GDV de objetos invariantes) |
| **Unroll Loops** | Só *unroll completo* (remove o loop) |
| **SSA + Value Numbering** | Base das otimizações seguintes |
| **LICM** (Loop Invariant Code Hoisting) | Move para o pré-header expressões invariantes sem efeitos colaterais |
| **Copy propagation** | Substitui variáveis com o mesmo VN |
| **Redundant Branch Opts** | Dobra condições conhecidas, `jump threading` |
| **CSE** | Elimina subexpressões redundantes usando VN |
| **Assertion propagation** | Propaga fatos como não-nulidade |
| **Range analysis** | Elimina checagens de limites de arrays |
| **Induction variable opts** | Strength reduction, loops contados para baixo, alargar IVs de 32 para 64 bits, remover IVs não usadas |
| **VN-based dead store elimination** | Remove stores que não mudam o valor |
| **If conversion** | Condicional vira `GT_SELECT` (cmov) |
| **Lowering + LSRA** | Seleção específica do alvo; alocação de registradores por *linear scan* |

Observação do doc sobre a estrutura: o JIT exige IR em **SSA convencional**, "não há tradução
*out of SSA*" (`optVnCopyProp`). Também há arquivos específicos: `loopcloning.cpp`,
`inductionvariableopts.cpp`, `rangecheck.cpp`, `rangecheckcloning.cpp`, `redundantbranchopts.cpp`,
`assertionprop.cpp`, `valuenum.cpp`, `copyprop.cpp`, `ifconversion.cpp`, `optimizebools.cpp`,
`switchrecognition.cpp`, `forwardsub.cpp`, `promotion.cpp`, `lsra*.cpp`.

### Otimizações dependentes de tier

Em Tier0 (`opts.OptimizationDisabled()`), a maior parte disso é desligada; por exemplo,
`fgUnpinNonMovableLocals` retorna `MODIFIED_NOTHING` se a otimização está desabilitada
(`lclmorph.cpp:~2460`). O doc de OSR (4.6) propõe algumas otimizações baratas em Tier0
(poda de ramos mortos, devirtualização) desde que não alterem o conjunto de variáveis vivas
nos patchpoints.

### Intrínsecos e SIMD

`importervectorization.cpp` e `importercalls.cpp` tratam métodos "conhecidos" (intrínsecos) como
chamadas especiais; `docs/design/coreclr/botr/vectors-and-intrinsics.md` descreve a integração
com `Vector<T>` e instruções da CPU. Em R2R, o JIT não pode assumir a CPU final, por isso o
doc de tiered compilation fala em reotimizar métodos R2R quentes.

## Por quê

- **Inlining é a otimização habilitadora**: remove custo de chamada e expõe constantes,
  tipos exatos e padrões de acesso ao resto do pipeline (`inlining-plans.md`: "directly reducing
  call overhead and indirectly exposing a wider scope for powerful intraprocedural
  optimizations").
- **Limites de tamanho/profundidade/orçamento** controlam **explosão de código e de tempo de
  JIT**, algo crítico para um JIT (ver arquivo 05).
- **Observações sobre argumentos** tornam a heurística orientada a benefício: o inline vale
  mais quando o caller passa uma constante.
- **PGO** (arquivo 06) fornece frequência de blocos e tipos prováveis, base de `ProfilePolicy`
  e GDV.
- **SSA + VN** dão base comum para várias otimizações com custo próximo do linear, alinhado ao
  princípio de boa vazão do JIT.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Inline agressivo | Menos chamadas, mais otimização | Código maior, mais tempo de JIT, mais pressão de registradores, pior I-cache |
| Orçamento de inlining | Teto no custo de compilação | Pode deixar de inlinar chamadas importantes depois de gastar o orçamento |
| Limite de 100 bytes IL | Simples e barato | Funções grandes quentes não inlinam (usa-se `AggressiveInlining` manualmente) |
| Heurísticas por observação | Boa qualidade sem execução | Difícil de ajustar; vários modelos de política (Default, Model, Profile) |
| Criar novo `Compiler` para o inlinee | Isolamento | Custo de memória/tempo por inline |
| GDV | Torna virtuais inlináveis | Código maior; depende de perfil confiável; guard falha custa |
| Clone de loops | Remove checks no caminho quente | Duplica código |
| Unroll só completo | Simplicidade | Perde ganhos de unroll parcial |
| Sem out-of-SSA | Simplicidade | Restrições de manter IR em SSA convencional |
| Otimizações super-lineares evitadas | JIT rápido | Algumas análises mais fortes ficam fora (ex.: interprocedural) |

## Como aplicar na sua linguagem

1. **Planeje inlining desde a IR**: a representação de chamada deve permitir "importar" o
   corpo do callee dentro do caller, mapeando argumentos, locais e retorno. Prefira
   fazer a decisão em duas etapas: **legalidade** (regras fixas do runtime) e
   **lucratividade** (heurística).
2. **Registre observações** (como `inline.def`): uma lista estruturada de fatos sobre o callee
   e o call site, cada um com um impacto. Isso facilita depurar por que algo não inlinou e
   treinar modelos depois.
3. **Imponha limites** de tamanho do callee, profundidade e orçamento total, e deixe um
   atributo de escape (`AggressiveInlining`/`NoInlining`) para o autor do código.
4. **Dê peso a argumentos constantes e tipos exatos**: são os maiores desbloqueadores.
5. **Se sua linguagem tem despacho virtual**, investimento em devirtualização (estática +
   guardada por perfil) rende muito; o .NET precisou de GDV porque a análise estática pegava
   pouco.
6. **Adote SSA + numeração de valores** cedo: a maior parte da lista (CSE, copy prop,
   assertion prop, range checks, redundant branches) é implementada sobre isso.
7. **Faça a forma canônica de loop** (pré-header, header dominando o corpo): destrava LICM,
   clone e unroll.
8. **Elimine checagens de limites** com análise de intervalo + clone de loop. Para linguagens
   com bounds checking obrigatório, isso é o que faz arrays serem competitivos.
9. **Separe otimizações por tier**: em Tier0 desligue quase tudo; ligue o pipeline completo
   apenas no tier final.
10. **Meça throughput do compilador** (tempo por fase). O RyuJIT tem timers por fase.
    Sem isso, o custo do JIT pode desfazer o ganho do código gerado.
11. Considere uma **política de inlining substituível** (classe `InlinePolicy` com várias
    implementações): permite experimentar modelos sem reescrever o inliner.

## Referências

- `docs/design/coreclr/jit/ryujit-overview.md` - fases, inlining, loops, SSA/VN e otimizações.
- `docs/design/coreclr/jit/inlining-plans.md`, `inline-size-estimates.md`.
- `docs/design/coreclr/jit/GuardedDevirtualization.md`.
- `docs/design/coreclr/jit/JitOptimizerPlanningGuide.md`, `JitOptimizerTodoAssessment.md`.
- `src/coreclr/jit/fginline.cpp`, `inline.cpp`, `inline.h`, `inline.def`, `inlinepolicy.{h,cpp}`.
- `src/coreclr/jit/compiler.h` (limites de inlining, linhas ~12666-12678).
- `src/coreclr/jit/jitconfigvalues.h` (`JitInlineSize`, `JitInlineDepth`, `JitInlineBudget`).
- `src/coreclr/jit/loopcloning.cpp`, `inductionvariableopts.cpp`, `rangecheck.cpp`,
  `redundantbranchopts.cpp`, `assertionprop.cpp`, `valuenum.cpp`, `ifconversion.cpp`,
  `forwardsub.cpp`, `promotion.cpp`, `lsra.cpp`, `compphases.h`.
- `docs/design/coreclr/botr/vectors-and-intrinsics.md`.
