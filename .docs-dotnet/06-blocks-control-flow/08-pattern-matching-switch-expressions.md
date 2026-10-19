# Pattern matching e switch expressions

Pattern matching e `switch` (instrução e expressão) **não têm instruções próprias no IL**. O IL oferece três ingredientes: a instrução `switch` (tabela de saltos indexada por inteiro), testes de tipo (`isinst`, `castclass`, `unbox.any`) e branches condicionais comparando inteiros/constantes. O compilador de linguagem decompõe padrões em uma árvore de decisão feita desses ingredientes; o JIT então reconhece e reotimiza formas de switch (jump table, bit test, peeling por PGO). Este documento mostra o que o runtime fornece, e o que o JIT faz depois.

## Mecanismo no .NET

### 1. A instrução `switch` do IL

`src/coreclr/inc/opcode.def`, linha 106:

```
OPDEF(CEE_SWITCH, "switch", PopI, Push0, InlineSwitch, IPrimitive, 1, 0xFF, 0x45, COND_BRANCH)
```

Formato (confirmado no leitor do JIT em `src/coreclr/jit/fgbasic.cpp`, `fgFindJumpTargets`, ~linhas 1923-1975): o operando é um `uint32` com o número de casos N, seguido de N deslocamentos de 32 bits **relativos ao fim da instrução** (`jmpBase`, o fim da tabela). O próprio `jmpBase` é marcado como alvo: "jmpBase is also the target of the default case". Há validações `BADCODE` ("jump target out of range") e exige-se que haja código depois do switch. Semântica: o valor do topo da pilha (`PopI`) indexa a tabela; fora do intervalo `[0, N)` o controle **cai para a instrução seguinte** (o caso `default` é a continuação física; a ECMA-335 define o índice como sem sinal, o que não é verificado neste trecho do JIT). Evidência no construtor de basic blocks: "Default case of CEE_SWITCH (next block), is at end of cases[]" e `noway_assert(curBBdesc->NextIs(cases[numCases - 1]->getDestinationBlock()))` (~linha 2843).

Não há `switch` sobre strings, tipos, ranges nem guardas no IL. Tudo isso é decomposto antes de chegar ao runtime.

### 2. Testes de tipo e extração

- `isinst <T>`: devolve o objeto ou `null` (`importer.cpp` ~linhas 3180 e 10445); `castclass <T>`: lança `InvalidCastException`; `unbox.any`.
- No runtime, estes viram chamadas a helpers em `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs`: `IsInstanceOfAny`, `IsInstanceOfInterface`, `IsInstanceOfClass`, `ChkCastAny`, `ChkCastInterface`, `ChkCastClass` (todos com variantes `*_NoCacheLookup` para o caminho lento, que consulta um cache de casts).
- O importador também **dobra padrões** de IL: por exemplo, `BOX` seguido de `ISINST` que sabidamente dá `null` é importado direto como `null` ("Importing BOX; ISINST; as null", `importer.cpp` ~linha 3195). Isso mostra que o JIT já espera sequências geradas por padrões como `x is T`.

Um padrão `x is Foo f && f.Y > 3` torna-se, em IL, algo como: `isinst Foo; dup; stloc f; brfalse L_false; ldloc f; ldfld Y; ldc.i4.3; ble L_false; ...` (forma ilustrativa; a sequência exata depende do compilador).

### 3. O que o JIT faz com `switch`

**a) Representação.** O bloco que termina em `switch` tem `BBJ_SWITCH` (`src/coreclr/jit/block.h`, linha 76). Os alvos ficam em `GetSwitchTargets()`, que pode ter um *dominant case* (usado com PGO).

**b) Reconhecimento de cadeias if/else como switch.** `Compiler::optRecognizeAndOptimizeSwitchJumps` (`src/coreclr/jit/switchrecognition.cpp`):

> Optimize range check for `x == cns1 || x == cns2 || x == cns3 ...` pattern and convert it to a BBJ_SWITCH block (jump table), which then *might* be converted to a bitmap test via TryLowerSwitchToBitTest. If we have PGO data, try peeling switches with dominant cases. TODO: recognize general jump table patterns.

Constantes de heurística no topo do arquivo: `SWITCH_MAX_DISTANCE = (TARGET_POINTER_SIZE * BITS_PER_BYTE) - 1` (a distância entre constantes tem de caber numa palavra), `SWITCH_MIN_TESTS` 3 (5 em ARM64, onde instruções condicionais podem ser melhores) e `CONVERT_SWITCH_TO_CCMP_MIN_TEST 5`. A rotina só olha blocos `BBJ_COND` ("for better throughput"), pula blocos "fallthrough" vazios (`SkipFallthroughBlocks`) respeitando a mesma região EH (`BasicBlock::sameEHRegion`) e detecta ciclos.

**c) Peeling por PGO.** `Compiler::fgPeelSwitch` (`src/coreclr/jit/fgopt.cpp`, ~linha 2913): "Modify a switch to check for its dominant case up front". Só ocorre com dados de perfil ("We currently will only see dominant cases with PGO"): o bloco vira um teste do caso dominante seguido de um novo `BBJ_SWITCH` sem caso dominante.

**d) Lowering.** `Lowering::LowerSwitch` (`src/coreclr/jit/lower.cpp`, linha 865). O comentário no topo (linhas ~820-860) descreve duas expansões:
   1. *Jump table*: um bloco `BBJ_COND` testa se o índice cai no `default` (`GT_JTRUE` com `GT_GE` contra o índice do default), e um novo bloco contém `GT_SWITCH_TABLE(localVar, jumpTable)`.
   2. *Switches pequenos*: sequência de comparações `if (case == first) goto ...; else if ...`. A decisão é `useJumpSequence = jumpCnt < minSwitchTabJumpCnt` (`lower.cpp` ~linhas 954-1023), com `minSwitchTabJumpCnt` começando em 2 ("table is better than just 2 cmp/jcc") e ajustado por plataforma.
   3. *Bit test*: `Lowering::TryLowerSwitchToBitTest` (~linha 1371): "If the jump table contains less than 32 (64 on 64-bit targets) entries and there are at most 2 distinct jump targets then the jump table can be converted to a word of bits"; emite `bt` + `jc` em vez do salto indireto, "both shorter and faster (in part due to the removal of a memory load)".

**e) Inlining.** O scanner de IL marca `CEE_SWITCH` como observação de inlining (`CALLEE_HAS_SWITCH`) e nota `CALLSITE_FOLDABLE_SWITCH` quando o argumento é constante (`fgbasic.cpp` ~1925-1930), isto é, um switch sobre argumento constante pode ser eliminado após o inline.

### 4. EH e switch

Alvos de `switch` não podem atravessar fronteiras de região protegida: branches só ficam dentro da mesma região (a única forma de sair é `leave`). Isso é imposto pela estrutura de regiões (doc 2) e é o motivo da checagem `sameEHRegion` nas otimizações.

### 5. O que o compilador C# faz (convenção; fora do repositório)

Comportamentos conhecidos do compilador, que **não podem ser confirmados aqui**:
- `switch` de inteiros densos vira `switch` do IL; esparsos, cadeia de comparações/busca binária.
- `switch` de string vira cálculo de hash seguido de comparações (uma função auxiliar `ComputeStringHash` gerada no `<PrivateImplementationDetails>`; uma busca por `ComputeStringHash` nos fontes deste repositório não encontrou nada além do compilador, que está ausente).
- Padrões de tipo viram `isinst`/`castclass` mais testes; padrões de propriedade viram leituras de campo/propriedade; padrões relacionais e lógicos (`> 3 and < 10`) viram comparações.
- A `switch expression` que não casa nenhum braço lança `SwitchExpressionException` (tipo existente na BCL em `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/SwitchExpressionException.cs`; que o compilador o lance nesse caso é convenção).
- A ordem de avaliação, a eliminação de testes redundantes e a checagem de exaustividade são feitas no front-end.

## Por quê

1. **Minimalismo do IL.** Manter só `switch` por inteiro e testes de tipo evita que o formato binário tenha de acompanhar a evolução de padrões da linguagem (C# 7 a 13 acrescentaram padrões sem mudar o IL).
2. **Otimização baseada em forma.** Como o JIT reconhece formas (`x == c1 || x == c2` vira `switch`, `switch` vira bit test), o compilador pode emitir a forma mais simples e ainda obter código bom. A divisão de trabalho: front-end decide a **semântica e a ordem**; JIT decide a **representação** (tabela, sequência, bit test).
3. **PGO.** Dados de perfil só existem em tempo de execução; só o JIT pode fazer peeling pelo caso dominante.
4. **Segurança de tipo e verificação.** `isinst` e `castclass` têm semântica fixa e helpers otimizados com cache, independentemente do padrão que os motivou.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| `switch` do IL só por inteiro sem sinal | Simples de verificar e de compilar | Strings, tipos e ranges precisam ser decompostos pelo front-end (hash + comparações) |
| Default = fall-through do `switch` | Compacto | Obriga layout cuidadoso dos blocos; o JIT assume que o default é o próximo bloco |
| Árvore de decisão no front-end | Ordem e efeitos colaterais controlados pela linguagem | Cada linguagem reimplementa a otimização de árvore de decisão |
| JIT reconhece cadeias de comparações | Código bom mesmo com IL ingênuo | Heurísticas dependentes da plataforma (ARM64 vs x64); throughput de JIT (só olha `BBJ_COND`) |
| PGO peeling | Caso comum é rápido | Só com perfil; complexidade adicional no fluxo |
| Testes de tipo via helpers | Casts rápidos com cache | Chamada de helper em casos não inlináveis; classes sealed permitem teste direto |
| Sem exaustividade no runtime | Runtime simples | Falta de braço gera exceção em tempo de execução, não erro de verificação |

## Como aplicar na sua linguagem

1. **Defina um bytecode de multiway pequeno**: `switch` por inteiro denso com default por queda (como o .NET) ou um `br_table` explícito com alvo default. Documente se o índice é sem sinal.
2. **Faça a compilação de padrões no front-end** usando uma árvore de decisão (algoritmo clássico de Maranget / "decision trees" com heurísticas). Meça o número de testes por caminho, e evite reavaliar subexpressões.
3. **Tenha primitivas de teste de tipo baratas**: `is_instance(T) -> ref|null`, `cast(T)`, e deixe o runtime ter cache de casts, como `CastHelpers`. Isso permite padrões de tipo sem custo proibitivo.
4. **Faça o otimizador reconhecer formas**: converter `x == a || x == b || x == c` em switch/bit test é ganho real (o JIT do .NET usa constantes pequenas como `SWITCH_MIN_TESTS`). Se sua linguagem não tem JIT, considere fazer isso no passo de lowering do compilador.
5. **Separe representação de semântica**: o IR deve poder trocar entre jump table, árvore de comparações e bit test sem mudar a semântica (o `LowerSwitch` do .NET faz isso depois do import).
6. **Garanta exaustividade em tempo de compilação** e defina o que acontece quando uma `switch expression` não casa (exceção específica, como `SwitchExpressionException` no .NET) para entradas inválidas em tempo de execução (por exemplo, enums com valores fora do conjunto).
7. **Respeite regiões protegidas**: um padrão com guarda (`when`) que chama código arbitrário que pode lançar precisa manter o fluxo dentro da mesma região; evite transformações que movam testes através de limites de `try`.
8. **Considere perfil**: se sua plataforma coletar perfis, ofereça um caminho de "caso dominante primeiro".
9. **Para strings**: decida entre hash+comparação, trie ou perfect hash. O .NET deixa isso ao front-end; documente a complexidade esperada para evitar surpresas.

## Apêndice: exemplo de decomposição (ilustrativo)

Padrão fonte (pseudo-C#):

```csharp
string Descreve(object o) => o switch
{
    null            => "nulo",
    int n when n > 0 => "positivo",
    int             => "inteiro",
    string { Length: 0 } => "vazio",
    string s        => s,
    _               => "outro",
};
```

Uma árvore de decisão possível, escrita como IL conceitual (a ordem e o compartilhamento de testes são decisões do front-end):

```
    ldarg o ; brfalse L_nulo          // null
    ldarg o ; isinst System.Int32 ; brfalse L_naoInt
    ldarg o ; unbox.any int32 ; stloc n
    ldloc n ; ldc.i4.0 ; ble L_inteiro   // guarda `when n > 0`
    ldstr "positivo" ; br L_fim
L_inteiro:  ldstr "inteiro" ; br L_fim
L_naoInt:
    ldarg o ; isinst System.String ; stloc s ; ldloc s ; brfalse L_outro
    ldloc s ; callvirt get_Length ; brtrue L_str
    ldstr "vazio" ; br L_fim
L_str:  ldloc s ; br L_fim
L_outro: ldstr "outro"
L_fim:  ret
```

O que o runtime enxerga: apenas `brfalse`, `isinst`, `unbox.any`, `ble`, `ldstr`, `br`, `ret`. Nenhuma instrução de "padrão". Pontos de atenção do JIT:
- `isinst` + `unbox.any` repetidos sobre o mesmo objeto são custo que o front-end deve minimizar compartilhando testes (o importador do JIT só dobra casos específicos, como `BOX; ISINST` conhecido);
- os `brfalse` encadeados formam blocos `BBJ_COND`; se fossem comparações de constantes inteiras, `optRecognizeAndOptimizeSwitchJumps` poderia agrupá-las em `BBJ_SWITCH`;
- a ordem dos braços afeta quais testes são executados primeiro; é decisão semântica do front-end (padrões devem ser testados na ordem textual, a menos que provadamente equivalente).

## Apêndice: perguntas de design

1. **Exaustividade**: obrigatória (erro de compilação) ou checada em runtime? No .NET há uma exceção de runtime, `SwitchExpressionException`, para o caso de nenhum braço casar.
2. **Ordem de avaliação**: padrões são testados na ordem textual? Guardas (`when`) podem ter efeitos colaterais? O front-end deve preservar a ordem observável.
3. **Reutilização de testes**: `x is Foo` repetido em vários braços deve virar um único `isinst` (compartilhamento na árvore de decisão).
4. **Custo de teste de tipo**: classes seladas e tipos valor permitem comparação direta de ponteiro de tipo; interfaces e classes abertas exigem helper com cache (`CastHelpers`).
5. **Switch sobre strings**: hash, trie ou comparação sequencial. Qual a complexidade garantida?
6. **Representação do `default`**: queda para a instrução seguinte (IL) ou alvo explícito.
7. **Otimização no backend vs. no front-end**: o JIT do .NET reconhece cadeias de `==` e as converte em `switch`/bit test; sem JIT, faça isso no seu compilador.
8. **Evolução da linguagem**: manter o bytecode pequeno (só multiway + testes de tipo) permitiu ao C# adicionar padrões em várias versões sem tocar o IL.

## Referências

- `src/coreclr/inc/opcode.def` (`switch`, linha 106; `isinst`/`castclass`).
- `src/coreclr/jit/fgbasic.cpp` (leitura de `CEE_SWITCH`, ~1923-1960; construção de blocos, ~2843).
- `src/coreclr/jit/block.h` (`BBJ_SWITCH`).
- `src/coreclr/jit/switchrecognition.cpp` (`optRecognizeAndOptimizeSwitchJumps`, constantes `SWITCH_*`).
- `src/coreclr/jit/fgopt.cpp` (`fgPeelSwitch`, ~2913).
- `src/coreclr/jit/lower.cpp` (`LowerSwitch`, ~865; `TryLowerSwitchToBitTest`, ~1371).
- `src/coreclr/jit/importer.cpp` (`BOX`+`ISINST`, `CEE_ISINST`, `CEE_CASTCLASS`).
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs`.

> Nota de confiança: toda a seção sobre como o compilador C# decompõe padrões é convenção do compilador (não presente neste repositório) e foi mantida em termos gerais. O comportamento de `switch`/JIT é confirmado nos arquivos citados.
