# Contextos checked/unchecked (overflow)

Em C#, `checked { ... }` e `unchecked { ... }` (e a opção de compilador de contexto padrão) parecem **escopos** que mudam o comportamento da aritmética inteira. No IL, esse escopo **não existe**: a escolha é feita **por instrução**. Existem duas famílias de opcodes, uma que dá a volta (wrap-around) e outra que lança `OverflowException`. O compilador de linguagem resolve o contexto léxico e escolhe o opcode. O JIT apenas implementa as duas famílias.

## Mecanismo no .NET

### 1. Duas famílias de opcodes

De `src/coreclr/inc/opcode.def`:

| Operação | Wrap (unchecked) | Overflow-checked | Linhas |
|---|---|---|---|
| soma | `add` | `add.ovf`, `add.ovf.un` | 251-252 |
| subtração | `sub` | `sub.ovf`, `sub.ovf.un` | 255-256 |
| multiplicação | `mul` | `mul.ovf`, `mul.ovf.un` | 253-254 |
| conversão para inteiro | `conv.i1`, `conv.u4`, ... | `conv.ovf.i1` ... `conv.ovf.u`; `conv.ovf.i1.un` ... `conv.ovf.u.un` | 167-176, 216-223, 249-250 |

O sufixo `.un` indica que os operandos são interpretados como **sem sinal** (na ECMA-335, no `conv.ovf.*.un` o valor de origem é tratado como sem sinal). Não há `div.ovf`, `neg.ovf` nem `shl.ovf` no IL. Logo, a checagem de overflow existe só em `+`, `-`, `*` e nas conversões. A divisão `INT_MIN / -1` e o `% ` têm tratamento próprio (exceção aritmética), independente do contexto.

Observação importante para design: **o operador unário `-x`** em contexto checked é, no compilador, implementado como `0 - x` com `sub.ovf` (convenção do compilador; não há `neg.ovf` em `opcode.def`, o que é verificável).

### 2. Como o JIT trata a família `.ovf`

`src/coreclr/jit/importer.cpp` (~linhas 7603-7650). Cada opcode mapeia para um nó `GT_ADD`/`GT_SUB`/`GT_MUL`, marcado com overflow:

```cpp
case CEE_ADD_OVF:    uns = false; goto ADD_OVF;
case CEE_ADD_OVF_UN: uns = true;  goto ADD_OVF;
ADD_OVF: ovfl = true; callNode = false; oper = GT_ADD; goto MATH_OP2_FLAGS;
...
MUL_OVF: ovfl = true; oper = GT_MUL; goto MATH_MAYBE_CALL_OVF;
```

Para multiplicação existe `MATH_MAYBE_CALL_OVF`, que em alvos onde `long * long` não tem instrução nativa pode virar chamada a helper (`CORINFO_HELP_LMUL_OVF`, `CORINFO_HELP_ULMUL_OVF` em `src/coreclr/inc/corinfo.h`, linhas 326-327; para double para inteiro: `CORINFO_HELP_DBL2INT_OVF`, linha 336).

A informação "checked" vai para a flag de árvore `GTF_OVERFLOW` (`src/coreclr/jit/gentree.h`, linha 524: "Supported for: GT_ADD, GT_SUB, GT_MUL and GT_CAST."), com os acessores `gtSetOverflow`/`gtClearOverflow` (linhas ~2269-2275) e o comentário de `GenTreeCast`: `"gtOverflow" (the "GTF_OVERFLOW" flag) - whether the cast is checked`. Casts checked são representados como `GT_CAST` com `GTF_OVERFLOW`.

### 3. Geração de código: desvio para um "bloco de throw"

Quando o nó é emitido, o codegen compara flags do processador e desvia para um **bloco auxiliar compartilhado** que chama o helper de throw. Em `gentree.h` (linhas ~45-70) o enum `SpecialCodeKind` lista esses alvos:

```
SCK_RNGCHK_FAIL, SCK_DIV_BY_ZERO, SCK_ARITH_EXCPN, SCK_OVERFLOW = SCK_ARITH_EXCPN,
SCK_ARG_EXCPN, SCK_ARG_RNG_EXCPN, SCK_FAIL_FAST, SCK_NULL_CHECK (Wasm)
```

O comentário diz: "SCK_OVERFLOW == SCK_ARITH_EXCPN" e "it's not clear why", ou seja, overflow e exceção aritmética compartilham o mesmo bloco. Exemplos de uso: `genJumpToThrowHlpBlk(EJ_jne, SCK_OVERFLOW)` em `codegenxarch.cpp` (~linha 6668) e `codegenarm.cpp` (~1396-1414), `codegenarmarch.cpp` (~3597).

O helper chamado é `CORINFO_HELP_OVERFLOW` ("throw an overflow exception", `corinfo.h`, linha 402). No lado gerenciado, as exceções são criadas por `ThrowHelper.ThrowOverflowException` (`src/libraries/System.Private.CoreLib/src/System/ThrowHelper.cs`, ~linha 258).

Consequências relevantes:
- Os blocos de throw são **compartilhados** por método, para reduzir o tamanho do código (marcados `BBF_THROW_HELPER` em `block.h`, linha 449).
- Como o desvio é para um bloco que lança, a operação `.ovf` é uma **operação com efeito colateral (pode lançar)**: o JIT não pode removê-la nem reordená-la livremente com outros efeitos; mas pode eliminá-la quando prova que não estoura (constant folding: `fgMorphCastIntoHelper` em `morph.cpp`, ~linha 107, "Tries to fold constants and also adds an edge for overflow exception").
- No grafo de fluxo, a possibilidade de lançar adiciona **arestas excepcionais**; dentro de um `try`, a exceção é tratada normalmente pelas duas passadas (doc 3).

### 4. Unchecked: o padrão

Sem `.ovf`, a aritmética de inteiro faz wrap em complemento de dois. `BitConverter.cs` mostra uso explícito de `unchecked(...)` em código de biblioteca (por exemplo `unchecked((char)ToInt16(...))`, linha 432; `unchecked((uint)startIndex) >= unchecked((uint)value.Length)`, linhas ~461-537, uma idiom clássica de **checagem de limites com uma comparação** usando a conversão sem sinal). Este idioma é um bom exemplo de por que `unchecked` precisa estar disponível localmente.

### 5. O contexto padrão

Qual contexto vale quando o código não declara nada é uma opção do compilador de linguagem (`CheckForOverflowUnderflow` em MSBuild para C#). Uma busca por esse nome nos arquivos `.props`/`.targets`/`.csproj` deste repositório não encontrou referências, ou seja, as bibliotecas do runtime usam o padrão (unchecked) e marcam `checked(...)` ou usam helpers (`Math.*`, `checked` explícito) onde precisam. A definição do padrão é do compilador, e não pode ser confirmada aqui.

### 6. Operações checked da BCL como alternativa

Para código genérico, a biblioteca expõe a escolha como **operadores sobrecarregáveis**: `IAdditionOperators<TSelf,TOther,TResult>` (`src/libraries/System.Private.CoreLib/src/System/Numerics/IAdditionOperators.cs`, linha 24) declara `static virtual TResult operator checked +(TSelf left, TOther right) => left + right;`, ou seja, uma versão `checked` do operador com implementação padrão igual à versão normal. E `int.CreateChecked<TOther>(TOther)` (`Int32.cs`, linha ~700) é a conversão checked genérica. Isso mostra a forma "API" do mesmo conceito: o contexto continua sendo escolhido por operação, agora também em código genérico.

## Por quê

1. **Granularidade e simplicidade.** Colocar a escolha no opcode evita estado dinâmico ("modo overflow") que o JIT teria de rastrear através de chamadas, inlining e exceções. A mesma instrução sempre faz a mesma coisa.
2. **Inlining sem surpresa.** Se o contexto fosse estado do chamador, inlinar um método num contexto diferente mudaria sua semântica. Com opcodes, o método já carrega sua semântica.
3. **Desempenho por padrão.** O wrap-around mapeia para a instrução nativa. O checked tem custo (flag + desvio), pago só onde o programador pede.
4. **Segurança como opt-in local.** Código que lida com tamanhos/offsets pode usar `checked` onde um overflow é um bug de segurança (alocação, cálculo de índices), sem pagar em tudo.
5. **Casts.** A conversão também é uma fonte de overflow; a família `conv.ovf.*` permite que `checked((byte)x)` seja uma única instrução.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Contexto resolvido no compilador, por opcode | JIT simples; semântica local e estável | Bibliotecas genéricas não herdam contexto do chamador; um método não pode ser "checked ou não" dinamicamente |
| Wrap como padrão | Máxima velocidade; sem exceções inesperadas | Overflows silenciosos causam bugs de segurança; é preciso lembrar de usar `checked` |
| Overflow = exceção | Falha explícita e capturável | Custo de ramo + bloco de throw; atrapalha vetorização e certas otimizações |
| Ausência de `neg.ovf`, `shl.ovf`, `div.ovf` | Conjunto de opcodes menor | O compilador deve compor sequências (subtração a partir de zero); `INT_MIN / -1` é caso separado |
| Blocos de throw compartilhados | Menos código | Stack trace aponta para o bloco compartilhado no método (o ponto exato do overflow vem da tabela de depuração do IP) |
| `.un` separado de signed | Semântica exata de sem sinal | Dobra a quantidade de opcodes; confusão entre "sem sinal" e "tipo sem sinal" |
| Casts checked separados | `checked` cobre conversões | Tabela de conversões grande (`conv.ovf.*`, 20+ opcodes) |

## Como aplicar na sua linguagem

1. **Decida onde mora a política de overflow**:
   - *por operação* (como o IL): operadores diferentes (`+` vs `+%`/`+|` à Zig/Rust: `wrapping_add`, `checked_add`, `saturating_add`);
   - *por escopo* (como `checked {}` do C#): açúcar do front-end que escolhe a operação.
   Em ambos os casos, **o IR deve carregar a escolha em cada instrução**, nunca em estado dinâmico.
2. **Escolha o padrão com cuidado.** O padrão do .NET é wrap. Para uma linguagem nova considere o contrário em modo debug (trap on overflow) e wrap/saturate em release (como Rust), documentando claramente.
3. **Ofereça as três semânticas** (wrap, trap/throw, saturate) e, opcionalmente, "overflow flag" (retorno `(valor, bool)`), pois o .NET só tem as duas primeiras no IL e delega o resto à biblioteca.
4. **Defina a exceção/erro**: tipo, mensagem, e se pode ser capturado. O .NET usa `OverflowException`, criada por um helper de runtime chamado de blocos compartilhados.
5. **Defina cobertura**: quais operadores têm variante checked (o .NET: `+ - *` e conversões; não negação/shift/divisão). Se sua linguagem incluir `neg`, `shl` e `abs`, lembre-se de `-INT_MIN`.
6. **Trate constantes em tempo de compilação**: o .NET dobra constantes e, se overflow ocorrer em contexto checked, gera erro do compilador ou a instrução com `.ovf` que lançará (convenção do C#; a parte do JIT é `fgMorphCastIntoHelper`/folding).
7. **Modele o efeito colateral no IR**: operações checked são "pode lançar" e têm arestas excepcionais; otimizações (CSE, hoisting) devem respeitar isso.
8. **Permita inlining/generics sem herdar contexto**: se sua linguagem tem genéricos sobre números, decida se `T + T` é checked ou não, e forneça operadores explícitos (`CheckedAdd`) como o .NET fez nas interfaces numéricas.
9. **Considere o idiom de verificação de limites** `(uint)i < (uint)len`, que depende de conversões que fazem wrap; se sua linguagem proíbe wrap por padrão, esse idioma precisa de uma forma explícita.

## Apêndice: exemplo de tradução

Fonte (C#-like):

```csharp
int a = x + y;                     // contexto padrão (unchecked): wrap
int b = checked(x + y);            // checked: lança OverflowException
int c = unchecked((int)bigLong);   // trunca
int d = checked((int)bigLong);     // lança se não couber
```

IL esperado (a seleção de opcode é do compilador; os opcodes existem em `opcode.def`):

```
ldloc x ; ldloc y ; add                // a
ldloc x ; ldloc y ; add.ovf            // b
ldloc bigLong ; conv.i4                // c
ldloc bigLong ; conv.ovf.i4            // d
```

No JIT, `add.ovf` e `conv.ovf.i4` viram `GT_ADD`/`GT_CAST` com `GTF_OVERFLOW`; no codegen x64 espera-se que a operação seja seguida de um desvio condicional sobre as flags do processador para o bloco de throw compartilhado (`genJumpToThrowHlpBlk(..., SCK_OVERFLOW)`), que chama `CORINFO_HELP_OVERFLOW`. (A instrução exata varia por arquitetura; ver `genJumpToThrowHlpBlk` nos `codegen*.cpp`.)

## Apêndice: tabela de decisão para sua linguagem

| Pergunta | Opção A (estilo .NET) | Opção B (estilo Rust/Zig) |
|---|---|---|
| Padrão de `+` | wrap (release) | trap em debug, wrap em release (Rust) ou trap sempre (Zig) |
| Como pedir o outro | bloco `checked {}` ou `checked(expr)` | métodos/operadores explícitos: `wrapping_add`, `checked_add`, `saturating_add` |
| Onde fica a política no IR | no opcode (`add` vs `add.ovf`) | no opcode ou numa intrinsic |
| Em código genérico | operadores `checked` sobrecarregáveis (`operator checked +`) | traits (`CheckedAdd`) |
| Conversões | família `conv.ovf.*` | `try_from` / `as` com semântica documentada |
| Erro | exceção `OverflowException` | pânico / `Option` / `Result` |
| Constantes | erro do compilador no contexto checked (convenção C#) | erro do compilador (lint deny) |

Só a coluna A está documentada neste repositório; a coluna B é referência comparativa para ajudar sua decisão, sem ser afirmação sobre o runtime .NET.

## Apêndice: cuidados de implementação do backend

1. **Bloco de throw compartilhado**: mantenha um bloco de overflow por método (e por kind), como `SpecialCodeKind`, para reduzir tamanho de código. Isso afeta informações de depuração: o IP do `jo` permanece mapeado ao ponto fonte, mas o bloco de throw não.
2. **Efeitos colaterais**: marque a operação como "pode lançar" nas análises; ela não pode ser movida para além de um store observável nem eliminada se o resultado não for usado.
3. **Constant folding**: ao dobrar `checked(const1 + const2)` que estoura, não dobre; mantenha a operação para lançar em runtime (o `fgMorphCastIntoHelper` do JIT só dobra quando consegue e retorna a árvore original caso contrário).
4. **Vetorização**: operações com possibilidade de exceção em cada iteração tipicamente bloqueiam vetorização automática; ofereça intrínsecas de saturação para código vetorial.
5. **64 bits em alvos de 32 bits**: multiplicação checked de `long` pode exigir helper (`CORINFO_HELP_LMUL_OVF`); preveja um caminho de biblioteca.

## Referências

- `src/coreclr/inc/opcode.def` (linhas 167-176, 216-223, 249-256: `conv.ovf.*`, `add.ovf`, `sub.ovf`, `mul.ovf` e variantes `.un`).
- `src/coreclr/jit/importer.cpp` (~7603-7650: `CEE_ADD_OVF`, `CEE_SUB_OVF`, `CEE_MUL_OVF`; rótulos `MATH_OP2_FLAGS`, `MATH_MAYBE_CALL_OVF`).
- `src/coreclr/jit/gentree.h` (`GTF_OVERFLOW`, linha 524; `SpecialCodeKind`/`SCK_OVERFLOW`, linhas ~45-70; comentário de `GenTreeCast`, ~4122).
- `src/coreclr/jit/morph.cpp` (`fgMorphCastIntoHelper`, ~107; tratamento de `gtOverflow()` ~294-316).
- `src/coreclr/jit/codegenxarch.cpp` (~6668), `codegenarm.cpp` (~1396-1414), `codegenarmarch.cpp` (~3597): `genJumpToThrowHlpBlk(..., SCK_OVERFLOW)`.
- `src/coreclr/jit/block.h` (`BBF_THROW_HELPER`).
- `src/coreclr/inc/corinfo.h` (`CORINFO_HELP_OVERFLOW` 402, `CORINFO_HELP_LMUL_OVF` 326, `CORINFO_HELP_ULMUL_OVF` 327, `CORINFO_HELP_DBL2INT_OVF` 336).
- `src/libraries/System.Private.CoreLib/src/System/ThrowHelper.cs` (`ThrowOverflowException`), `BitConverter.cs` (`unchecked(...)`).

> Nota de confiança: a escolha de contexto padrão e a expansão exata de `checked {}` são do compilador C#, não do runtime; neste repositório estão confirmados os opcodes `.ovf`, o tratamento no JIT e o helper de throw.
