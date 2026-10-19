# Checagem de overflow e de limites de array (bounds check)

> Escopo: o `dotnet/runtime` não contém o compilador C#, então a semântica `checked`/`unchecked` da linguagem é descrita apenas no nível em que se materializa em IL (opcodes `*.ovf`) e no JIT. O foco é: como o runtime **impõe** a checagem em tempo de execução e como o JIT **prova que ela é desnecessária** (análise estática de intervalos).

## Mecanismo no .NET

### 1. Duas checagens implícitas, duas famílias de exceção

| Checagem | Origem em IL | Exceção em runtime |
|---|---|---|
| Limites de array | `ldelem*`, `stelem*`, `ldelema` (checam índice contra `ldlen`) | `IndexOutOfRangeException` |
| Overflow aritmético | `add.ovf`, `add.ovf.un`, `sub.ovf`, `sub.ovf.un`, `mul.ovf`, `mul.ovf.un`, `conv.ovf.*` | `OverflowException` |

O verificador de IL reconhece esses opcodes: em `ILImporter.Verify.cs` (`ImportBinaryOperation`, ~linha 2138) os casos `add_ovf`, `add_ovf_un`, `mul_ovf`, `mul_ovf_un`, `sub_ovf`, `sub_ovf_un` aparecem **no grupo que exige operandos inteiros** (`CheckIsInteger`), enquanto `add/sub/mul/div/rem` comuns aceitam também floats (`CheckIsNumeric`). Razão semântica: overflow de ponto flutuante gera ±Inf, não exceção.

Ou seja, a escolha entre `checked` e `unchecked` é feita **no momento da emissão de IL** (o compilador de linguagem escolhe `add` vs `add.ovf`); o runtime não tem modo global.

### 2. Como o JIT representa as checagens

Em `src/coreclr/jit/gentree.h`:

- `GTF_OVERFLOW = 0x10000000` — "Suportado para: GT_ADD, GT_SUB, GT_MUL e GT_CAST" (linha ~524). `gtOverflow()` consulta a flag.
- `GTF_EXCEPT = 0x00000004` — "sub-expressão pode lançar exceção" (linha ~367). É isso que impede reordenar/remover indevidamente a operação: um nó que pode lançar tem efeito colateral observável.
- `GenTreeBoundsChk` (linha ~7917) representa `GT_BOUNDS_CHECK(index, length)`; o construtor define `gtFlags |= GTF_EXCEPT` e guarda um `SpecialCodeKind`.
- O enum `SpecialCodeKind` (linhas ~45-66) lista os "alvos de exceção" compartilhados por método: `SCK_RNGCHK_FAIL` ("alvo quando a checagem de limites falha"), `SCK_DIV_BY_ZERO`, `SCK_ARITH_EXCPN` com `SCK_OVERFLOW = SCK_ARITH_EXCPN`, `SCK_ARG_EXCPN`, `SCK_ARG_RNG_EXCPN`, `SCK_FAIL_FAST`, `SCK_NULL_CHECK` (Wasm). Há um comentário honesto no código: "não está claro por que SCK_OVERFLOW == SCK_ARITH_EXCPN".
- `Compiler::acdHelper` (`flowgraph.cpp` ~linha 3517) mapeia cada kind ao helper de lançamento: `SCK_RNGCHK_FAIL → CORINFO_HELP_RNGCHKFAIL`, `SCK_DIV_BY_ZERO → CORINFO_HELP_THROWDIVZERO`, `SCK_ARITH_EXCPN → CORINFO_HELP_OVERFLOW`, etc. As falhas desviam para blocos de lançamento (`BBJ_THROW`, `flowgraph.cpp` ~linha 3677) criados por "special code descriptor" (`AddCodeDsc`, chave com `acdKind` e `acdData`, `compiler.h` ~linha 7514), e o comentário do enum diz que são alvos "especiais (únicos)" por tipo; não verifiquei os detalhes exatos de compartilhamento.

### 3. Expansão da indexação: `fgMorphIndexAddr`

`morph.cpp` (~linha 2880, função `fgMorphIndexAddr`) expande `GT_INDEX_ADDR`. Comentário do código (resumido):

- Em **minopts** o nó não é expandido, para reduzir o tamanho do IR.
- Ao **otimizar**, expande para: (1) avalia o array e guarda em temporário se complexo; (2) idem para o índice; (3) `GT_BOUNDS_CHECK(index, GT_ARR_LENGTH(array))`; (4) calcula o endereço do elemento; (5) envolve o endereço em `GT_ADD_ADDR` para uso do value numbering. "Essa expansão expõe explicitamente a checagem de limites e o cálculo de endereço ao otimizador".

Isso é uma decisão central: **a checagem vira um nó de IR comum**, sujeito às mesmas otimizações (CSE, hoisting, eliminação).

### 4. Eliminação por asserções: `optAssertionProp_BndsChk`

`assertionprop.cpp` (~linha 5430): "Dado uma árvore com bounds check, remova-a se já foi verificada no fluxo do programa". A função calcula, de forma preguiçosa, o intervalo do índice e do comprimento via `GetRange(..., assertions, /*fast*/ true)` e descarta a checagem (`dropBoundsCheck`, "Removing redundant (%s) bounds check") quando consegue provar `0 <= idx < len`. Usa value numbers (`optConservativeNormalVN`) para reconhecer a mesma expressão em outro ponto. Observação: em *local assertion prop* a otimização não ocorre ("não temos o tipo certo de asserções").

### 5. Análise de intervalos: `RangeCheck`

`rangecheck.h` descreve o algoritmo em comentário de cabeçalho, com o exemplo:

```
for (int i = 0; i < a.len; ++i) { a[i] = 0; }
```

Passos (do comentário original):
1. Percorrer instruções; para cada bounds check, perguntar o intervalo do índice.
2. Seguir *defs* e a cadeia de dependências em SSA (ex.: `i_1 = phi(i_0, i_2)`); se há ciclo, marcar o intervalo como "dependente".
3. Mesclar asserções das arestas dos predecessores.
4. **Verificar se ocorre overflow na cadeia de dependências** (laço): `DoesBinOpOverflow`, `DoesPhiOverflow`, `DoesVarDefOverflow`, `ComputeDoesOverflow`, `AddOverflows`, `MultiplyOverflows` (`rangecheck.cpp`, ~linhas 2042-2236). Sem isso, `i + 1` poderia dar a volta e invalidar a prova.
5. Checar monotonicidade e aplicar *widening* (assumir que variáveis dependentes herdam o limite inferior da definição fora do laço).

Aqui overflow e bounds check se encontram: **a prova de ausência de out-of-range só vale se a aritmética do índice não causa overflow**. Por isso o analisador de intervalos tem uma sub-análise de overflow e usa `CheckedOps::MulOverflows` / `IntAddOverflows`.

### 6. Clonagem de checagens: `rangecheckcloning`

`rangecheckcloning.cpp/.h` implementa a fase "Range check cloning": para sequências como `arr[i+1]; arr[i+3]; arr[i+5]; arr[i+8]` que o assertion prop não consegue provar, a fase agrupa checagens por par (VN do índice-base, VN do comprimento) e clona o bloco em caminho rápido/lento:

```
if (i >= 0 && i < arr.Length - 8) { /* rápido: sem bounds check */ }
else                                { /* lento: com bounds check */ }
```

Constantes: `MIN_CHECKS_PER_GROUP 4`, `MAX_CHECKS_PER_GROUP 64`, `BUDGET_MULTIPLIER 40` (limite de complexidade). É o equivalente em arrays do *loop cloning*.

### 7. Constant folding de aritmética checada

Em `gentree.cpp`, `gtFoldExprConst*` (veja arquivo 09) trata `GT_ADD/SUB/MUL/CAST` com `gtOverflow()`: se ambos operandos são constantes e `CheckedOps::AddOverflows(...)` (linhas ~18372-18396, ~18621-18639; para casts, `CastFromIntOverflows`, `CastFromLongOverflows`, `CastFromDoubleOverflows`) indica overflow, a operação **não** vira constante: chama `gtFoldExprForOverflow` (linha ~19000), que, só durante `fgGlobalMorph`, a transforma em chamada ao helper de overflow (a exceção continua ocorrendo em runtime, no ponto original). Comentário no código: "Esta operação vai causar uma exceção de overflow. Transforme em helper de overflow. Coloque um valor constante fictício para geração de código."

### 8. `ConstantExpectedAttribute` e `Debug.Assert` de limites

Em CoreLib (`System/Diagnostics/CodeAnalysis/ConstantExpectedAttribute.cs`) existe `ConstantExpectedAttribute`, que marca parâmetros que devem ser constantes (usado em hardware intrinsics) — uma "verificação" de outra natureza: o analisador e o JIT esperam um valor conhecido em tempo de compilação, sem o qual a otimização/semântica falham.

## Por quê

- **Segurança de memória**: bounds check elimina a classe inteira de buffer overflow em código gerenciado; é o pilar do "managed = memory safe".
- **Aritmética previsível**: C# dá `checked` para quem precisa detectar overflow e `unchecked` (default) para quem quer envolver (*wrap*) com custo zero.
- **Desempenho sem sacrificar segurança**: o custo da checagem seria alto em loops; tornar a checagem um nó otimizável (`GT_BOUNDS_CHECK`) permite provar redundância e removê-la.
- **Exceções precisas**: as flags `GTF_EXCEPT`/`SpecialCodeKind` preservam a ordem observável de exceções quando a checagem *não* pode ser removida.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Checagem sempre ligada para arrays | Segurança sem depender do programador | Custo em loops quentes; dependência de otimizador |
| `unchecked` como padrão da linguagem (decisão da linguagem; `add` vs `add.ovf` no IL) | Desempenho previsível | Overflow silencioso por padrão; bugs de segurança de lógica |
| Checagem como nó de IR | Otimizável (CSE, VN, assertion prop, range analysis, cloning) | IR maior; complexidade de efeitos de exceção |
| Análise de range com análise de overflow integrada | Prova correta mesmo com `i+1` | Custo de compilação; limites (`MAX_CHECKS_PER_GROUP`, `BUDGET_MULTIPLIER`) |
| Cloning (fast/slow path) | Remove muitas checagens em padrões comuns | Aumenta tamanho de código; heurísticas de orçamento |
| Blocos de lançamento "especiais" por kind (`SpecialCodeKind`) | Código menor: o caminho frio de falha sai do fluxo principal | Complexidade extra para manter a semântica de exceção |
| Em minopts, não expandir | Compilação rápida | Código mais lento |

## Como aplicar na sua linguagem

1. **Decida a política de overflow**: (a) wrap por padrão + opt-in `checked` (C#), (b) trap por padrão + opt-in wrap (Rust em debug, Swift), (c) tipos distintos (`Wrapping<T>`, `Checked<T>`). Seja explícito no IR: instruções separadas (`add`/`add.ovf`) permitem ao otimizador raciocinar.
2. **Torne bounds checks nós de IR primeiro-classe** com efeitos de exceção marcados (`GTF_EXCEPT`) e um *kind* de falha que mapeia para um helper.
3. **Faça análise de intervalos em SSA** com: asserções vindas de condições (`i < n`), propagação via `phi`, detecção de monotonicidade em laço e **sub-análise de overflow** da cadeia de dependências. Sem a análise de overflow a prova é incorreta.
4. **Implemente constant folding que respeita overflow**: quando a operação checada com constantes overflowa, **preserve a exceção** (não dobre para valor); só dobre quando seguro.
5. **Considere cloning de checagens** (caminho rápido/lento com uma guarda) para sequências de acessos com offsets constantes.
6. **Ofereça APIs que expressem invariantes ao otimizador**: iteradores sobre span, `for (T x in array)` — o JIT reconhece o padrão; considere `ConstantExpected`-like para parâmetros que precisam ser constantes.
7. **Tenha um nível de otimização mínimo** que não expande tudo (como minopts) para compilação rápida.
8. **Defina a mensagem/tipo de exceção por kind** e teste a *ordem* de exceções (se a linguagem promete exceções precisas).

## Apêndice: exemplos de raciocínio e checklist

### Exemplo 1: laço canônico (do comentário de `rangecheck.h`)

```
for (int i = 0; i < a.len; ++i) { a[i] = 0; }
```

SSA do comentário: `i_0 = 0; i_1 = phi(i_0, i_2); a[i_1] = 0; i_2 = i_1 + 1;` com condição `i_1 < a.len`.

Prova: limite inferior vem de `i_0 = 0` + monotonicidade crescente (cada iteração soma 1); limite superior vem da aresta condicional `i_1 < a.len`; ausência de overflow em `i_2 = i_1 + 1` é garantida porque `i_1 < a.len ≤ int.MaxValue`, logo `i_1 + 1 ≤ int.MaxValue`. Resultado: o `GT_BOUNDS_CHECK` do corpo é removido.

### Exemplo 2: onde falha e entra o cloning

```
arr[i + 1] = x; arr[i + 3] = y; arr[i + 5] = z; arr[i + 8] = w;
```

Cada acesso exige `i+k < len` com `k` crescente; as asserções não oferecem prova. A fase de *range check cloning* agrupa as 4 checagens (o mínimo `MIN_CHECKS_PER_GROUP` é 4), calcula a guarda `i >= 0 && i < arr.Length - 8` e duplica o trecho em caminhos rápido (sem checagem) e lento (com checagem).

### Exemplo 3: aritmética checada com constantes

`checked(int.MaxValue + 1)` em IL é `ldc.i4 0x7fffffff; ldc.i4.1; add.ovf`. O folding vê `GT_ADD` com `gtOverflow()` e constantes; `CheckedOps::AddOverflows` retorna verdadeiro; em vez de dobrar para `int.MinValue`, chama `gtFoldExprForOverflow`, que em `fgGlobalMorph` produz uma chamada ao helper de overflow — a exceção continua sendo lançada em runtime no ponto correto, e o código seguinte pode ser tratado como inalcançável.

### Tabela: opcodes IL e checagens

| Opcode | Efeito | Exceção |
|---|---|---|
| `add`, `sub`, `mul` | aritmética com wrap (inteiros) / IEEE (floats) | nenhuma |
| `add.ovf`, `sub.ovf`, `mul.ovf` (+`.un`) | falha se resultado não cabe no tipo | `OverflowException` |
| `conv.ovf.*` (+`.un`) | conversão que falha se valor fora do alvo | `OverflowException` |
| `div`, `rem` | — | `DivideByZeroException` (e overflow em `MIN/-1`) via `SCK_DIV_BY_ZERO`/`SCK_ARITH_EXCPN` |
| `ldelem*`, `stelem*`, `ldelema` | acesso a elemento | `IndexOutOfRangeException` via `SCK_RNGCHK_FAIL` |
| `ldlen` | comprimento do array | `NullReferenceException` se nulo |

(As linhas de `div`/`rem` e `ldlen` seguem a semântica geral do IL e os kinds de `SpecialCodeKind`; não verifiquei cada caminho do JIT para eles.)

### Checklist de projeto

1. Quem decide `checked`/`unchecked`: fonte, opção do compilador (`CheckOverflow` está em `CompilationData` do RegexGenerator, o que mostra que o gerador precisa conhecer essa opção) ou tipo?
2. A falha é exceção, trap, ou valor saturado?
3. Existe forma de provar ausência de falha ao otimizador? (condições de laço, `assume`, tipos refinados)
4. Há um caminho lento compartilhado para falhas (helper) que não polui o caminho quente?
5. O verificador de bytecode exige operandos inteiros nas variantes checadas (como `CheckIsInteger` no ILVerify)?

## Referências

- `src/coreclr/jit/gentree.h` (`GTF_OVERFLOW`, `GTF_EXCEPT`, `SpecialCodeKind`, `GenTreeBoundsChk`)
- `src/coreclr/jit/flowgraph.cpp` (`acdHelper`, blocos de lançamento)
- `src/coreclr/jit/morph.cpp` (`fgMorphIndexAddr`)
- `src/coreclr/jit/assertionprop.cpp` (`optAssertionProp_BndsChk`)
- `src/coreclr/jit/rangecheck.h`, `rangecheck.cpp`, `rangecheckcloning.h`, `rangecheckcloning.cpp`
- `src/coreclr/jit/gentree.cpp` (`gtFoldExprConst`, `gtFoldExprForOverflow`, `CheckedOps::*`)
- `src/coreclr/tools/ILVerification/ILImporter.Verify.cs` (`ImportBinaryOperation`)
- `docs/design/coreclr/jit/ryujit-overview.md` (seção Range Analysis)
- `src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/ConstantExpectedAttribute.cs`
