# Basic blocks, branches e goto no IL

O IL do .NET tem **um único mecanismo de fluxo de controle intra-método**: branches para offsets de IL. Não há `if`, `while`, `for`, `break`, `continue` nem `goto` como conceitos distintos: todos viram `br`, `brtrue`/`brfalse`, os branches de comparação (`beq`, `blt`, ...) ou `switch`. O JIT reconstrói a estrutura (basic blocks, grafo de fluxo, loops naturais) a partir disso. Para quem projeta uma linguagem, o ponto-chave é: um bytecode de **grafo de fluxo com destinos explícitos**, restrito apenas por regras de pilha e por regiões protegidas.

## Mecanismo no .NET

### 1. As instruções de salto

Em `src/coreclr/inc/opcode.def` (linhas 80-105 e 106, 257-259):

| Grupo | Opcodes | Notas |
|---|---|---|
| Incondicional | `br`, `br.s` | `InlineBrTarget` (32 bits) ou `ShortInlineBrTarget` (8 bits), deslocamento relativo |
| Teste de valor | `brfalse`, `brtrue` (e `.s`) | `PopI`: um inteiro/ref/ponteiro |
| Comparação | `beq`, `bge`, `bgt`, `ble`, `blt`, `bne.un`, `bge.un`, `bgt.un`, `ble.un`, `blt.un` (e `.s`) | `Pop1+Pop1`; versões `.un` para sem sinal/NaN |
| Multi-via | `switch` | doc 8 |
| Saída de região | `leave`, `leave.s` | esvazia pilha; ver doc 2 |
| Término | `ret`, `throw`, `rethrow`, `endfinally`, `endfilter`, `jmp` | sem sucessores no método |

Cada salto tem uma versão curta (`.s`) e longa. O offset é **relativo ao fim da instrução**, e o alvo é um offset de IL (como se vê na decodificação do `switch` em `fgbasic.cpp`, onde `jmpAddr = jmpBase + getI4LittleEndian(codeAddr)`). Não existem rótulos simbólicos no formato binário.

### 2. Do IL a basic blocks no JIT

O JIT constrói o grafo em duas passadas sobre o IL (`src/coreclr/jit/fgbasic.cpp`):

1. `Compiler::fgFindJumpTargets` (linha ~860): "walk the IL stream, determining jump target offsets". Preenche um bit vector `jumpTarget[N]`: 1 se o offset N de IL é alvo de salto. Também levanta observações para inlining e valida o IL ("May throw an exception if the IL is malformed"); por exemplo, alvos fora do método geram `BADCODE3("jump target out of range", ...)`.
2. `Compiler::fgMakeBasicBlocks` (linha ~2887): "walk the IL creating basic blocks". Cria um `BasicBlock` novo em cada alvo de salto, após cada instrução de salto/retorno/throw, e nos limites de regiões EH (e, em builds de debug, em limites de escopo: "we will also create blocks at scope boundaries" quando `opts.compDbgCode`).

Cada bloco termina com um "tipo de salto" `BBKinds` (`src/coreclr/jit/block.h`, linhas 62-76), que codifica o terminador:

```
BBJ_EHFINALLYRET, BBJ_EHFAULTRET, BBJ_EHFILTERRET, BBJ_EHCATCHRET,
BBJ_THROW, BBJ_RETURN, BBJ_ALWAYS, BBJ_LEAVE (só até o import),
BBJ_CALLFINALLY, BBJ_CALLFINALLYRET, BBJ_COND, BBJ_SWITCH
```

Mapeamento: `br` vira `BBJ_ALWAYS`; branches condicionais viram `BBJ_COND` (alvo + fall-through); `switch` vira `BBJ_SWITCH`; `ret` vira `BBJ_RETURN`; `throw` vira `BBJ_THROW`; `leave` vira `BBJ_LEAVE`, canonizado depois pelo `impImportLeave` em `BBJ_ALWAYS`/`BBJ_CALLFINALLY`/`BBJ_EHCATCHRET` (doc 2).

Flags de bloco relevantes (`block.h`, linhas 406-449): `BBF_BACKWARD_JUMP`, `BBF_BACKWARD_JUMP_SOURCE`, `BBF_BACKWARD_JUMP_TARGET` (detecção de loops por salto para trás), `BBF_HAS_LABEL`, `BBF_IMPORTED`, `BBF_INTERNAL` (bloco criado pelo compilador, não presente no IL), `BBF_DONT_REMOVE`, `BBF_ASYNC_RESUMPTION`, `BBF_THROW_HELPER`.

### 3. Loops não existem no IL: são recuperados

Um `for` e um `while` são o mesmo padrão de salto para trás. O JIT identifica **loops naturais** depois, a partir do grafo (`FlowGraphNaturalLoop`, `compiler.h` ~linha 2130). Um `goto` arbitrário que crie fluxo irredutível apenas impede certas otimizações, mas é válido. Salto para trás é detectado já na importação (`BBF_BACKWARD_JUMP_TARGET`) e usado, por exemplo, para decidir **patchpoints de OSR** (On-Stack Replacement): em `importer.cpp` (~linha 6281), `addPatchpoint = block->HasFlag(BBF_BACKWARD_JUMP_TARGET) && (stackState.esStackDepth == 0)`, e o comentário observa que loops cuja pilha IL não está vazia na aresta de retorno não podem ter patchpoint.

### 4. A pilha de avaliação e as fronteiras de bloco

O IL é uma máquina de pilha. As regras que o JIT impõe nas fronteiras de blocos (todas em `importer.cpp`):

- Pilha vazia na entrada de um bloco `try` (`BADCODE("Evaluation stack must be empty on entry into a try block")`, linha 12407).
- `localloc` só com pilha vazia (`"Localloc can only be used when the stack is empty"`, linha ~10345).
- Um bloco que termina em `ret`/`throw`/`endfinally`/etc. não pode ter pilha não vazia inalcançável (`"can't have 'unreached' end of BB with non-empty stack"`, ~linha 12632).
- Se a pilha não estiver vazia no fim de um bloco, o importador faz **spill** de cada entrada da pilha para temporários (`impSpillStackEntry`, rótulo `SPILLSTACK`), de modo que os blocos sucessores partam de variáveis locais comuns. Se o estado da pilha de dois predecessores for incompatível, o IL é rejeitado: `BADCODE("bad stack state")` (~linha 12777).

Isso é uma consequência direta de duas propriedades do bytecode: (a) a pilha é parte do estado do ponto de programa; (b) qualquer ponto de junção deve ter uma pilha de forma única (verificado pelo importador, como no verificador ECMA).

### 5. Restrições estruturais que `goto` respeita

O `br` do IL é "livre" apenas dentro de uma região:

- Não se pode entrar numa região protegida pelo meio; apenas pelo primeiro bloco do `try`.
- Não se sai de `try`/`catch` por `br`; apenas por `leave` (doc 2).
- `filter`/`finally`/`fault` não aceitam saltos externos.
- Mesmo com `leave`, o JIT precisa de blocos "step" para atravessar regiões aninhadas (`impImportLeave`).

Por isso o `goto` de C# que sai de `try/finally` vira `leave` e chama o finally; e `goto case` vira simples `br`.

### 6. Blocos e outras fases do JIT

Depois do import o JIT limpa e reordena blocos: remove inalcançáveis, junta blocos, reordena para layout, separa hot/cold (`docs/design/coreclr/jit/hot-cold-splitting.md`), e usa as arestas (`FlowEdge`) com probabilidades de PGO. `docs/design/coreclr/jit/ryujit-overview.md` e `ryujit-tutorial.md` descrevem a visão geral das fases; `BasicBlock` é a estrutura base em `block.h`. Blocos "retless" (`BBF_RETLESS_CALL`) modelam `BBJ_CALLFINALLY` que nunca retorna.

## Por quê

1. **Máximo denominador comum entre linguagens.** Qualquer estrutura de controle (laços, `break`, `continue`, `goto`, `match`, corrotinas após lowering) pode ser traduzida para saltos; o runtime não precisa conhecer cada uma.
2. **Verificação simples.** O verificador/JIT basta rastrear: alvos válidos, forma da pilha em junções e aninhamento de regiões.
3. **Otimização.** Basic blocks e arestas são a unidade básica de dataflow, layout e alocação de registradores; recuperar loops naturais é um problema bem entendido.
4. **Compacidade.** Offsets relativos de 8 bits para saltos curtos reduzem o tamanho do IL.
5. **Regiões protegidas ortogonais.** Como EH é uma tabela separada (doc 2), os branches não carregam semântica de exceção.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Saltos para offsets absolutos/relativos | Formato simples e denso | Ferramentas precisam reconstruir blocos e rótulos; edição de IL exige recomputar offsets |
| Sem estrutura de laço no IL | Qualquer linguagem se encaixa | O JIT redescobre loops; `goto` irredutível limita otimizações |
| Pilha de avaliação | Código compacto | Spill obrigatório em junções; regras de pilha vazia (try, localloc, etc.) complicam geradores |
| Versões curta/longa de cada salto | IL menor | Emissor precisa de relaxamento de saltos (escolher curto vs. longo) |
| Blocos criados em cada alvo | Grafo exato | Muitos blocos pequenos; custo de compilação em métodos grandes |
| `switch` com default por queda | Compacto | Layout acoplado ao default |
| Regras de região nos saltos | Verificação trivial | Compiladores precisam inserir `leave`/blocos intermediários |

## Como aplicar na sua linguagem

1. **Escolha entre bytecode de grafo (como o IL/LLVM) ou estruturado (como WebAssembly: `block/loop/br`).** Estruturado garante redutibilidade e simplifica verificação e compilação em um passe; grafo é mais flexível para `goto` e otimizações de linguagem. O .NET escolheu grafo, restrito por regiões.
2. **Se escolher grafo, defina:**
   - alvos como rótulos simbólicos no formato textual, offsets no binário;
   - regras de pilha/registradores nas junções (pilha de mesma forma; ou, sem pilha, uso de registradores virtuais SSA/locals);
   - se saltos podem entrar no meio de regiões protegidas (não deveriam).
3. **Forneça um terminador explícito por bloco** (no estilo `BBKinds`): sempre ou condicional ou multi-via ou retorno ou throw. Isso evita "queda" implícita entre blocos e simplifica análise.
4. **Considere eliminar a pilha de avaliação** (usar registradores virtuais/locals) para evitar as regras especiais de "pilha vazia em try/localloc" e o custo de spill. O .NET carrega a pilha por herança de design.
5. **Reconheça loops no compilador, não no formato.** Identifique loops naturais a partir de dominadores; use saltos para trás como dica para contadores de recompilação/OSR (como `BBF_BACKWARD_JUMP_TARGET`).
6. **Defina o que é `goto` na sua linguagem**: o .NET expõe `goto` (C#) mas o restringe por escopo de região; documente restrições (não pular para dentro de blocos com inicialização, não sair de `finally`).
7. **Ofereça "rótulos de depuração" e mapeamento** entre blocos e posições fonte (o JIT cria blocos em limites de escopo em modo debug para mapear variáveis).
8. **Escolha cedo a codificação de saltos**: curto/longo e relaxação, ou apenas um tamanho fixo. Sem isso, um emissor de código vai ter de resolver o problema de relaxação de saltos.
9. **Valide no carregamento**: alvos dentro do método, nenhum salto para dentro de região, pilha consistente, todo caminho termina em instrução terminal ("Control is not permitted to simply fall through the end of a method", citado em `flowgraph.cpp` a partir da ECMA-335).

## Apêndice: exemplo de IL para um laço e seus blocos

Fonte (C#-like):

```csharp
int s = 0;
for (int i = 0; i < n; i++) { if (i % 2 == 0) continue; s += i; }
return s;
```

IL conceitual (a forma "laço com teste no fim", muito comum em compiladores):

```
        ldc.i4.0 ; stloc s
        ldc.i4.0 ; stloc i
        br   L_teste
L_corpo:
        ldloc i ; ldc.i4.2 ; rem ; brtrue L_soma   // i % 2 != 0 -> soma
        br   L_incr                                 // continue
L_soma: ldloc s ; ldloc i ; add ; stloc s
L_incr: ldloc i ; ldc.i4.1 ; add ; stloc i
L_teste:
        ldloc i ; ldarg n ; blt L_corpo             // salto para trás
        ldloc s ; ret
```

Divisão em basic blocks que `fgMakeBasicBlocks` produziria (um bloco por alvo de salto e após cada terminador):

| Bloco | Conteúdo | Terminador |
|---|---|---|
| B0 | inicializações | `BBJ_ALWAYS` para B_teste |
| B_corpo | `i % 2` e teste | `BBJ_COND` (alvo B_soma, queda B_continue) |
| B_continue | `br L_incr` | `BBJ_ALWAYS` |
| B_soma | `s += i` | queda para B_incr |
| B_incr | `i++` | queda para B_teste |
| B_teste | `i < n` | `BBJ_COND` (alvo B_corpo = salto para trás) |
| B_fim | `ldloc s; ret` | `BBJ_RETURN` |

Observações:
- `break` e `continue` são só `br` para rótulos diferentes. O JIT não sabe que existiam.
- O salto para trás de B_teste para B_corpo marca B_corpo com `BBF_BACKWARD_JUMP_TARGET` (candidato a patchpoint OSR se a pilha IL estiver vazia).
- Tudo o que o JIT sabe de "loop" vem depois, de dominadores (`FlowGraphNaturalLoop`).
- Esta divisão é ilustrativa; blocos vazios de "queda" podem ser mesclados pelas fases seguintes.

## Apêndice: checklist para um verificador de grafo de fluxo (baseado nas verificações do JIT)

1. Todo alvo de salto cai dentro do corpo do método (`"jump target out of range"`).
2. Alvo de salto é início de instrução (não meio de operando).
3. Todo caminho termina em `ret`, `throw`, `rethrow`, `endfinally`, `endfilter`, `jmp` ou `leave` (nada de queda no fim do método).
4. A forma da pilha é idêntica em todos os predecessores de um bloco (`"bad stack state"`).
5. Pilha vazia na entrada de `try` e antes de `leave`/`localloc`.
6. Entradas em região protegida só pelo início do `try`; saídas só via `leave`, `throw`, `endfinally`, `endfilter`.
7. `endfinally`/`endfilter` no nível de aninhamento correto (`"endfinally / endfault / endfilter in nested try block"`).
8. `rethrow` apenas dentro de `catch`.
9. Números de locais e argumentos dentro do intervalo declarado.
10. Tipos compatíveis nas junções (o verificador ECMA; o JIT assume IL válido ou verificado em outro lugar).

## Referências

- `src/coreclr/inc/opcode.def` (linhas 80-106: branches e `switch`; 257-259: `leave`).
- `src/coreclr/jit/block.h` (`BBKinds`, linhas 62-76; `BBF_*`, linhas 406-449).
- `src/coreclr/jit/fgbasic.cpp` (`fgFindJumpTargets` ~860, `fgMakeBasicBlocks` ~2887, `fgFindBasicBlocks` ~3377).
- `src/coreclr/jit/importer.cpp` (regras de pilha: 10345, 12407, 12632, 12777; patchpoints OSR ~6281; `impImportLeave` 4743).
- `src/coreclr/jit/compiler.h` (`FlowGraphNaturalLoop`, ~2130).
- `src/coreclr/jit/flowgraph.cpp` (comentário com citações da ECMA-335 sobre `ret` e fall-through).
- `docs/design/coreclr/jit/ryujit-overview.md`, `ryujit-tutorial.md`, `hot-cold-splitting.md`.
- `docs/design/specs/Ecma-335-Augments.md` (adendos à ECMA-335).
