# CIL (IL) — modelo de máquina de pilha de avaliação

## Mecanismo no .NET

O Common Intermediate Language (CIL, ou só IL) é o "assembly" portátil do .NET. Ele é definido em ECMA-335 (Partição III define as instruções; Partição I §12.3 define o modelo de execução). O compilador de linguagem (C#, F#, VB) emite IL; o runtime (JIT, Crossgen2, ILC) traduz IL para código nativo.

### Modelo: máquina de pilha

Cada método tem:

- uma sequência de bytes de IL (o *corpo*), armazenada fora das tabelas de metadata e referenciada pela coluna `RVA` da tabela `MethodDef`;
- uma **pilha de avaliação** (evaluation stack) — não existe acesso por registrador no IL;
- **argumentos** (`ldarg`/`starg`) e **variáveis locais** (`ldloc`/`stloc`), declarados por uma assinatura de locais (`LOCAL_SIG`, ver arquivo 04);
- uma tabela de cláusulas de exceção (`try`/`catch`/`filter`/`finally`/`fault`).

Cada instrução consome (*pop*) operandos da pilha e empilha (*push*) resultado. Exemplo conceitual de `a + b * 2`:

```
ldarg.0        // pilha: [a]
ldarg.1        // pilha: [a, b]
ldc.i4.2       // pilha: [a, b, 2]
mul            // pilha: [a, b*2]
add            // pilha: [a + b*2]
ret            // retorna o topo
```

### A tabela mestre de opcodes: `opcode.def`

O arquivo `src/coreclr/inc/opcode.def` é a "tabela mestre" da qual todas as listas de opcodes são derivadas (o próprio cabeçalho diz: *"This is the master table from which all opcode lists are derived"*). Ele contém 294 linhas `OPDEF`. O formato de cada linha é:

```
OPDEF(CEE_ADD, "add", Pop1+Pop1, Push1, InlineNone, IPrimitive, 1, 0xFF, 0x58, NEXT)
//    enum      nome   pop         push   operando    tipo        len byte1 byte2 fluxo
```

Campos relevantes:

| Campo | Significado | Exemplos reais em `opcode.def` |
|---|---|---|
| pop | O que sai da pilha | `Pop0`, `Pop1`, `Pop1+Pop1`, `PopI`, `PopRef`, `VarPop` |
| push | O que entra | `Push0`, `Push1`, `PushI`, `PushRef`, `VarPush` |
| operando | Formato do operando inline | `InlineNone`, `InlineMethod`, `InlineType`, `InlineBrTarget`, `InlineSwitch` |
| tipo | Categoria | `IPrimitive`, `IMacro`, `IObjModel`, `IPrefix` |
| len | Bytes do opcode (1 ou 2) | `add` = 1, `ldftn` = 2 |
| byte1/byte2 | Codificação | 1 byte: `0xFF, 0xNN`; 2 bytes: prefixo `0xFE` (`STP1`) + byte |
| fluxo | Efeito no controle de fluxo | `NEXT`, `BRANCH`, `COND_BRANCH`, `CALL`, `META` |

Codificações confirmadas no arquivo:

- `ldarg.0` = `0x02` (forma "macro", `IMacro`); `add` = `0x58`; `call` = `0x28`; `br` = `0x38`; `switch` = `0x45`; `callvirt` = `0x6F`; `box` = `0x8C`; `unbox.any` = `0xA5`.
- Opcodes de 2 bytes usam o prefixo `0xFE`: `ldftn` = `FE 06`, `tail.` = `FE 14`, `constrained.` = `FE 16`.
- `VarPop`/`VarPush` marcam `call`/`callvirt`/`newobj`: o efeito na pilha depende da assinatura do método chamado (tamanho variável).
- O cabeçalho define `RESERVED_PREFIX_START 0xF7`, e a categoria `IPrefix` agrupa instruções que modificam a *próxima* instrução (`tail.`, `constrained.`, `volatile.`, `unaligned.`, `readonly.`).

### Famílias de instruções

1. **Carga/armazenamento**: `ldarg`, `ldloc`, `ldc.*`, `ldstr`, `ldnull`, `starg`, `stloc`, e as variantes curtas (`ldarg.0`..`ldarg.3`, `ldarg.s`) — as formas curtas existem para reduzir tamanho de código.
2. **Aritmética/lógica**: `add`, `sub`, `mul`, `div`, `rem`, `and`, `or`, `xor`, `shl`, `shr`, e variantes `.ovf` (checadas).
3. **Conversões**: `conv.*`, `conv.ovf.*`. O repositório documenta em `docs/design/specs/Ecma-335-Augments.md` a seção "Unsigned data conversion with overflow detection".
4. **Controle de fluxo**: `br`, `brtrue`, `beq`, `blt`..., `switch` (tabela de saltos), `ret`, `leave`, `endfinally`, `endfilter`, `throw`, `rethrow`.
5. **Chamadas**: `call`, `callvirt`, `calli`, `newobj`, `ldftn`, `ldvirtftn`, `jmp`; prefixo `tail.` (chamada de cauda).
6. **Modelo de objeto**: `ldfld`, `stfld`, `ldsfld`, `ldflda`, `newarr`, `ldelem`, `stelem`, `ldlen`, `castclass`, `isinst`, `box`, `unbox`, `unbox.any`, `ldtoken`, `initobj`, `cpobj`, `sizeof`.
7. **Ponteiros/byrefs**: `ldind.*`, `stind.*`, `ldarga`, `ldloca`, `localloc`, `cpblk`, `initblk`.

### Tipos na pilha de avaliação

ECMA-335 I.12.3.2.1 define que a pilha só contém um conjunto reduzido de "tipos de pilha": `int32`, `int64`, `native int`, `F` (ponto flutuante), `O` (referência a objeto), `&` (ponteiro gerenciado/byref) e tipos-valor (structs). Tipos menores (`int8`, `int16`, `bool`, `char`) são **estendidos** para `int32` ao carregar e **truncados** ao armazenar. O addendum do repositório (`docs/design/specs/Ecma-335-Augments.md`) tem seção "Implicit argument coercion rules" e remove a menção a "transient pointers" da spec (seção "Transient pointers").

### Verificabilidade e forma da pilha

Uma propriedade essencial: em qualquer ponto do IL, a forma (profundidade e tipos) da pilha é determinável estaticamente. Mas o addendum do repositório diz que a restrição de "backward branch" (II.1.7.5) foi **removida** da spec porque nenhum runtime relevante a impunha ("it is not possible to infer the exact state of the evaluation stack at every instruction with a single forward-pass through the CIL instruction stream"). Ou seja: na prática, o JIT precisa de análise de fluxo (basic blocks) para reconstruir a pilha, e o IL "bem formado" exige que, em pontos de junção (merge), a pilha seja consistente entre predecessores.

### Como o IL vira código nativo

O JIT (RyuJIT; ver `docs/design/coreclr/jit/ryujit-overview.md`) importa a pilha de avaliação para uma IR em árvores (GenTree), simulando a pilha durante a importação: cada push vira um nó, cada pop consome nós. Locais/argumentos viram variáveis (`lclVar`) candidatas a registrador. Esse passo "stack → árvores → SSA/registradores" é o que torna a máquina de pilha barata de gerar mas ainda eficiente em código final. O ILC (Native AOT) e o Crossgen2 reutilizam o mesmo importador via `src/coreclr/tools/aot/jitinterface` e `ILCompiler.RyuJit`.

## Por quê

- **Portabilidade**: a pilha abstrai o número de registradores e o ABI do alvo. O mesmo `.dll` roda em x64, arm64, etc.
- **Compactação**: instruções sem operandos explícitos de registrador são pequenas (`add` = 1 byte). Há formas curtas (`ldarg.0`, `ldc.i4.s`, `br.s`) para os casos comuns.
- **Facilidade de geração por compiladores**: emitir IL para uma expressão é uma travessia pós-ordem da AST, sem alocação de registradores no front-end.
- **Verificabilidade**: tipos de pilha estáticos permitem validar tipo-segurança sem executar.
- **Instruções de alto nível**: `callvirt`, `newobj`, `box`, `castclass`, `ldfld` carregam semântica de modelo de objeto (nulos, despacho virtual, barreiras GC) que o runtime decide como implementar — isso é o que permite que R2R/AOT mudem layouts e estratégias de despacho sem mudar o IL (ver `docs/design/coreclr/botr/readytorun-overview.md`, seções "Instance Field access", "Vtable Dispatch").

## Trade-offs

| Vantagem | Custo |
|---|---|
| IL pequeno e portátil | O backend precisa reconstruir registradores/fluxo (JIT tem custo de tempo e energia; é uma das motivações do R2R — `readytorun-overview.md` "Motivation") |
| Semântica de objetos de alto nível | O IL não expressa tudo: otimizações (inlining, devirtualização) exigem conhecimento global/estático |
| Pilha dá forma única e simples | A "forma da pilha" em saltos para trás não é garantida pela spec moderna; ferramentas precisam de análise de fluxo (ver Augments) |
| Fácil emitir | Algumas construções (ex.: `localloc`, ponteiros não-gerenciados) quebram a verificabilidade |
| Extensível por prefixos | Prefixos (`tail.`, `constrained.`) criam acoplamento entre instruções adjacentes — ferramentas que reescrevem IL precisam preservá-los (Augments, seção "Rules for IL rewriters") |

Um custo concreto, documentado em `docs/design/coreclr/botr/readytorun-overview.md`: como o IL de `ldfld`/`newobj`/`callvirt` não fixa offsets nem layouts, o código pré-compilado precisa de indireções (fixups) quando cruza *version bubbles* — o IL abstrato é o que dá resiliência a versões, mas o nativo paga por isso.

## Como aplicar na sua linguagem

1. **Decida se sua IR é de pilha ou de registradores.** Pilha: mais simples de emitir e serializar; registradores/SSA: mais fácil de otimizar. O .NET prova que a pilha funciona se o backend importa para uma IR mais rica (JIT).
2. **Defina uma tabela mestre de instruções** como `opcode.def`: uma única fonte da verdade (nome, pop/push, operando, codificação, fluxo) da qual se geram o assembler, o desassemblador, o verificador e o importador. Copie as colunas `pop/push/operando/fluxo`.
3. **Codificação em 1 byte + prefixo de escape** (`0xFE`) para crescer sem quebrar. Reserve faixa de opcodes (como `RESERVED_PREFIX_START`).
4. **Tipos de pilha reduzidos**: poucos tipos (int32, int64, nativo, float, ref, byref, valor) e regras de extensão/truncamento explícitas.
5. **Mantenha instruções de modelo de objeto com semântica "de alto nível"** (`newobj`, `callvirt`, `ldfld` por token de campo, não por offset). Isso preserva liberdade de layout e viabiliza compilação antecipada resiliente a versões.
6. **Garanta invariantes de pilha em junções** (mesma profundidade/tipos de todos os predecessores), mesmo que você não proíba saltos para trás com pilha não vazia. Facilita o importador (uma passagem por bloco básico).
7. **Operandos por token**, não por endereço: `call <token>` referencia metadata (arquivo 02); isso desacopla IL de layout.
8. **Formas curtas** para casos frequentes, medindo frequência real de uso.
9. **Prefixos** para modificadores raros de instrução (chamada de cauda, constrained, volatile) em vez de multiplicar opcodes — mas documente que reescritores devem preservá-los.
10. **Chamadas com `VarPop`/`VarPush`**: o efeito na pilha deriva da assinatura do alvo; o verificador precisa resolver a assinatura (arquivo 04).

Exemplo mínimo de definição (inspirado em `opcode.def`) para uma linguagem nova:

```
OP(ADD,   "add",   pop=2, push=1, operand=none,   enc=0x58, flow=next)
OP(CALL,  "call",  pop=var, push=var, operand=method_token, enc=0x28, flow=call)
OP(BR,    "br",    pop=0, push=0, operand=branch32, enc=0x38, flow=branch)
OP(CONSTRAINED, "constrained.", pop=0, push=0, operand=type_token, enc=0xFE16, flow=meta, prefix=true)
```

## Referências

- `src/coreclr/inc/opcode.def` — tabela mestre de opcodes (294 linhas `OPDEF`).
- `src/coreclr/inc/corhdr.h` — tipos e flags de metadata relacionados (ver arquivos 02–04).
- `docs/design/specs/Ecma-335-Augments.md` — correções/adições à ECMA-335 (seções: "Backward branch constraints", "Transient pointers", "Creating arrays using newobj", "Implicit argument coercion rules", "Rules for IL rewriters").
- `docs/design/coreclr/jit/ryujit-overview.md` — importação IL → IR no RyuJIT.
- `docs/design/coreclr/botr/readytorun-overview.md` — como instruções do modelo de objeto são tratadas em código pré-compilado.
- ECMA-335, Partição I §12.3 (modelo de execução, pilha de avaliação) e Partição III (conjunto de instruções CIL).

## Apêndice A — Exemplo completo: `if/else` e chamada

C# (ilustrativo):

```csharp
static int Abs(int x) { if (x < 0) return -x; else return x; }
```

IL típico (forma otimizada de compilador; offsets em bytes; usa apenas opcodes presentes em `opcode.def`):

```
IL_0000: ldarg.0        // pilha: [x]                 (0x02)
IL_0001: ldc.i4.0       // pilha: [x, 0]
IL_0002: bge.s IL_0007  // pop 2; se x >= 0 salta     (branch condicional, COND_BRANCH)
IL_0004: ldarg.0        // pilha: [x]
IL_0005: neg            // pilha: [-x]
IL_0006: ret            // retorna o topo
IL_0007: ldarg.0        // pilha: [x]
IL_0008: ret
```

Pontos a observar:

- **A pilha está vazia em cada junção de blocos básicos.** Isto é o padrão dos compiladores e é o que `ICorDebugInfo::STACK_EMPTY_BOUNDARIES` explora para colocar pontos de parada seguros (ver arquivo 09).
- Os operandos de salto são **offsets relativos** (`InlineBrTarget`); `.s` indica a forma curta de 1 byte. O `switch` (`InlineSwitch`) carrega uma tabela de alvos.
- Nenhuma instrução menciona registradores nem tipos de `x` além do que a assinatura do método declara (arquivo 04). O tipo na pilha é `int32`.

Exemplo de chamada virtual com `VarPop`:

```
ldarg.0                // this (O)
ldarg.1                // argumento
callvirt <MemberRef/MethodDef token>   // pop = 1 (this) + nº de params, push = 0 ou 1 conforme o retorno
```

O efeito na pilha depende da assinatura do token: é por isso que o `opcode.def` marca `callvirt` com `VarPop`/`VarPush`.

## Apêndice B — Armadilhas de projeto de IL (lições visíveis no repositório)

1. **Dependências entre instruções (prefixos).** `constrained.` só é válido imediatamente antes de `callvirt`; `tail.` imediatamente antes de `call*` seguido de `ret`. Ferramentas que reordenam ou inserem instruções precisam respeitar isso (seção "Rules for IL rewriters" do `Ecma-335-Augments.md`).
2. **Pilha em saltos para trás.** A spec removeu a restrição de "backward branch" porque os runtimes reais não a aplicavam; seu importador deve calcular a forma da pilha por *fluxo de dados* sobre blocos, não por uma passagem linear.
3. **`newobj` vs `newarr`.** O Augments esclarece que arrays unidimensionais de base zero *devem* ser criados com `newarr`, mas não proíbe `newobj`, porque todo array tem construtores fornecidos pelo runtime.
4. **Tamanhos pequenos × `int32`.** Se sua linguagem tem inteiros pequenos, defina a regra de extensão ao carregar e truncamento ao armazenar (modelo `int32` na pilha).
5. **Endereços de locais.** Segundo o Augments, `ldarga`/`ldloca` produzem endereços de memória não gerenciada que podem ser convertidos em ponteiros sem fixar explicitamente — decida se sua linguagem permite isso (afeta GC e verificabilidade).
6. **Instruções "meta".** No `opcode.def`, `META` marca opcodes que não alteram o fluxo mas modificam a interpretação da próxima (prefixos).
