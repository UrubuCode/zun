# JIT: compilação método a método

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.

## Mecanismo no .NET

### Unidade de compilação = o método

O compilador JIT do .NET se chama **RyuJIT** (`src/coreclr/jit/`). Segundo
`docs/design/coreclr/jit/ryujit-overview.md`, `ICorJitCompiler::compileMethod()` "é invocado
para cada método, e cria um novo objeto `Compiler`. Assim, o JIT não precisa se preocupar com
sincronização de threads ao acessar o estado do `Compiler`". O EE (runtime) garante
sincronização: "há uma única cópia compilada de um método quando duas ou mais threads tentam
disparar a compilação do mesmo método".

Ou seja: o JIT é essencialmente **uma função pura-ish** `IL + informações do runtime -> código
nativo + GC info + EH info + debug info`, sem estado compartilhado entre compilações.

### A interface JIT <-> EE

Duas interfaces (ryujit-overview.md, "Execution Environment and External Interface"):

- `ICorJitCompiler` - implementada pelo JIT; definida em `src/coreclr/inc/corjit.h`,
  implementação em `src/coreclr/jit/ee_il_dll.cpp`. Métodos-chave: `compileMethod` e
  `getVersionIdentifier` (um GUID que versiona a interface JIT/EE).
- `ICorJitInfo` - implementada pelo runtime (`src/coreclr/vm/jitinterface.cpp`), herda de
  `ICorDynamicInfo` (`src/coreclr/inc/corinfo.h`). Permite ao JIT: resolver tokens de
  metadados, percorrer assinaturas de tipos, calcular offsets de campos e de vtable, achar
  entry points de métodos, construir literais de string etc.

O mesmo JIT serve JIT em tempo de execução **e compilação antecipada** (ryujit-overview:
"provides both just-in-time and ahead-of-time compilation service"), reaproveitando-se em
ReadyToRun/crossgen e Native AOT (a doc de R2R está em
`docs/design/coreclr/botr/readytorun-overview.md`). Também pode ser um binário separado do
runtime ("the EE and JIT may reside in the same or different executable files").

### Como um método chega ao JIT: precode e prestub

`docs/design/coreclr/botr/method-descriptor.md` ("Precode"):

1. Todo método tem, desde o início, um **entry point temporário** (precode) para que o código
   chamador tenha um endereço antes de o método ser compilado. Eles "precisam ser pequenos
   porque existem muitos, mesmo ao custo de performance"; são executados **uma só vez**.
2. O alvo do entry point temporário é o **PreStub**, "um tipo especial de stub que dispara o
   jitting de um método. Ele substitui atomicamente o entry point temporário por um entry
   point estável".
3. O entry point estável é o código nativo (JIT ou pré-compilado) ou o precode.

No código: `MethodDesc::PrepareInitialCode` (`src/coreclr/vm/prestub.cpp:310`) cria um
`PrepareCodeConfig` e chama `PrepareCode`; o caminho de IL chama `JitCompileCode`
(`prestub.cpp:604`, "a forma thread-safe de invocar o compilador JIT").

Resultado: **compilação preguiçosa (lazy), por método, na primeira chamada**. Métodos nunca
chamados nunca são compilados.

### IR e fases do RyuJIT

Estrutura interna (ryujit-overview.md): uma função é uma lista duplamente ligada de
`BasicBlock`, com arestas explícitas de fluxo normal; o fluxo excepcional é implícito, descrito
por uma tabela de regiões `EHblkDsc`. O JIT tem duas formas de IR:

- **HIR** (alto nível): árvores `GenTree` em `Statement`s; usada no front end.
- **LIR** (baixo nível): lista linear de nós; criada na fase de *rationalization*.

Há também SSA e *value numbering* (seções "SSA" e "Value Numbering" do mesmo doc).

A ordem das fases está em `src/coreclr/jit/compphases.h` (macro `CompPhaseNameMacro`).
Resumo (nomes reais do arquivo): Pre-import -> Importation -> Indirect call transform ->
Expand patchpoints -> Post-import -> Profile instrumentation/incorporation -> Morph - Inlining
-> Allocate Objects (escape analysis) -> Remove empty try/finally -> Local morph -> Early
liveness -> Physical promotion -> Forward substitution -> Morph global -> Invert loops ->
Optimize layout -> Find/Clone/Unroll loops -> Hoist loop code -> Build SSA -> Value numbering
-> Optimize index checks -> CSE -> Redundant branch opts -> Assertion prop -> Range check
cloning -> If conversion -> VN-based dead store removal -> (back end) rationalization,
lowering, LSRA (register allocation), codegen.

Tabela da doc (`ryujit-overview.md`, "Phases of RyuJIT") em alto nível:

| Fase | Resultado |
|---|---|
| Importation | IL vira árvores `GenTree`; candidatos a inline identificados |
| Inlining | IR do callee é incorporada ao fluxo do caller |
| Object Stack Allocation | Análise de escape; objetos na pilha (arquivo 09) |
| Struct promotion / Physical promotion | Campos de structs viram locais separados |
| Morph (local/global) | Normalização obrigatória + otimizações simples |
| Loops (find/invert/clone/unroll) | Forma canônica `do..while`, clonagem para eliminar checks |
| SSA + VN | Base para LICM, CSE, copy prop, assertion prop, range analysis, IV opts |
| Rationalization | HIR -> LIR |
| Lowering | Ajustes específicos do alvo |
| Register allocation | Linear scan (LSRA) |
| Code generation | Frame, prólogo/epílogo, código, GC/EH/debug info |

### Princípios de projeto do RyuJIT (citados do doc)

- Alta compatibilidade com JITs anteriores (x86 jit32, x64 jit64).
- Boa performance de código via otimizações, alocação de registradores e codegen.
- **Boa vazão (throughput) do JIT** "por meio de otimizações e transformações em grande parte
  de ordem linear, junto a limites no número de variáveis rastreadas para análises de
  dataflow que são inerentemente super-lineares".
- Arquitetura que suporte um espectro de alvos e cenários.

O limite de variáveis rastreadas é um exemplo concreto de **trocar qualidade por tempo de
compilação**, porque o JIT paga seu custo durante a execução do programa.

### Tiers e flags

O runtime pede ao JIT diferentes "modos" por meio de flags em
`src/coreclr/inc/corjitflags.h`: `CORJIT_FLAG_TIER0` ("gerar código o mais rápido possível"),
`CORJIT_FLAG_TIER1` ("código de alta qualidade"), `CORJIT_FLAG_OSR` ("versão alternativa para
On Stack Replacement"), `CORJIT_FLAG_BBINSTR` ("coletar perfil de blocos básicos"). Ver
arquivo 06.

### Informação que o JIT devolve ao runtime

Além do código: **GC info** (quais slots/registradores contêm referências em cada ponto
seguro; `gcencode.cpp`), **EH info**, **debug info** (`ryujit-overview.md`, "Supporting
technologies"). GC info é o que permite o GC mover objetos referenciados por frames
compilados.

## Por quê

- **Preguiça**: a maioria do código em aplicativos grandes nunca roda ou roda uma vez;
  compilar tudo antecipadamente desperdiça tempo e memória. O próprio doc de tiered compilation
  (`docs/design/features/tiered-compilation.md`) fala de mitigar problemas de *startup* por
  quase 20 anos com NGEN, ReadyToRun e MulticoreJit.
- **Informação de execução**: o JIT enxerga o processador real (suporte a instruções) e as
  dependências carregadas, e - com tiering - perfis de execução. Isso permite gerar código
  melhor que um compilador AOT genérico (`tiered-compilation.md`, "Steady-State": "At runtime
  the JIT is able to observe the exact dependencies that are loaded as well as CPU instruction
  support").
- **Isolamento por método** simplifica concorrência e permite compilar em várias threads.
- **Interface JIT/EE estreita e versionada** permite trocar o JIT, ou reutilizá-lo em outras
  situações (AOT), sem acoplar com o resto do runtime.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Compilar por método, sob demanda | Startup melhor; só paga pelo que usa | Latência no primeiro uso; sem visão do programa inteiro |
| Visão local de método (+ inlining) | Simplicidade, paralelismo | Análises interprocedurais limitadas (veja escape analysis, arquivo 09) |
| Precode/prestub para cada método | Chamadas sempre têm destino válido | Overhead de indireção e memória por método; mecanismo de backpatching |
| JIT gasta tempo durante a execução | Código adaptado ao hardware | Tempo de JIT conta no tempo do app; por isso otimizações têm que ser quase lineares |
| Interface JIT/EE enorme (`ICorJitInfo`) | Desacopla JIT do runtime | Interface grande para manter e versionar (GUID) |
| JIT reutilizado para AOT | Um único backend | Restrições de versionamento (R2R não pode depender de layout de classes de outros assemblies - ver `object-stack-allocation.md`) |

## Como aplicar na sua linguagem

1. **Decida sua estratégia de execução**: interpretador, JIT, AOT, ou mix. O .NET é um mix:
   JIT por padrão, R2R/AOT para startup. Se sua linguagem exige rapidez de partida, planeje
   um caminho pré-compilado desde cedo.
2. **Se escolher JIT, defina a unidade como a função/método**, com estado de compilação
   privado por invocação (`Compiler` objeto novo a cada método). Isso permite compilar em
   paralelo e simplifica a robustez.
3. **Crie uma fronteira estreita runtime <-> compilador** (um `ICorJitInfo` próprio): o
   compilador pergunta ao runtime sobre tipos, campos, vtables, e o runtime pergunta o
   mínimo. Versione essa interface.
4. **Implemente "entry points temporários"**: um stub por função que, na primeira chamada,
   compila e troca o destino atomicamente. Esse mecanismo também será a base para tiering
   (arquivo 06), já que o mesmo stub pode contar chamadas.
5. **Projete a IR com HIR (árvores) e LIR (lista linear)** se pretende fazer otimizações
   fortes e alocação de registradores; ou comece com uma IR única mais simples. O RyuJIT
   evoluiu do jit32, então a divisão HIR/LIR é herança; **[inferido]** que uma linguagem nova
   poderia começar com SSA direto.
6. **Limite análises super-lineares** (número de variáveis rastreadas, tamanho de método) para
   proteger o tempo de JIT. Registre contadores de tempo por fase (o JIT tem `measureIR`/
   timing por fase em `compphases.h`).
7. **Emita metadados para o GC desde o início**: mapa de referências por ponto seguro (GC
   info). Sem isso, GC preciso (e com movimento) não funciona.
8. **Pense em depuração e diagnóstico**: o JIT do .NET tem `JitDump` (ver
   `docs/design/coreclr/jit/viewing-jit-dumps.md`) e tem compilação com `Debug` flags. Uma
   linguagem nova ganha muito com um dump textual de IR por fase.

## Referências

- `docs/design/coreclr/jit/ryujit-overview.md` - estrutura do JIT, IR, fases, GC info, EH.
- `docs/design/coreclr/botr/method-descriptor.md` - seção "Precode" (entry points
  temporários, PreStub).
- `docs/design/coreclr/botr/readytorun-overview.md` - código pré-compilado (R2R).
- `docs/design/features/tiered-compilation.md` - contexto histórico (NGEN, R2R, MulticoreJit).
- `src/coreclr/jit/compphases.h` - lista e ordem das fases.
- `src/coreclr/jit/ee_il_dll.cpp`, `src/coreclr/inc/corjit.h` - `ICorJitCompiler`.
- `src/coreclr/inc/corinfo.h`, `src/coreclr/vm/jitinterface.cpp` - `ICorJitInfo`/`ICorDynamicInfo`.
- `src/coreclr/inc/corjitflags.h` - `CORJIT_FLAG_TIER0/TIER1/OSR/BBINSTR`.
- `src/coreclr/vm/prestub.cpp` - `PrepareInitialCode`, `JitCompileCode`.
- `src/coreclr/jit/gcencode.cpp` - codificação de GC info.
- `docs/design/coreclr/jit/viewing-jit-dumps.md`, `ryujit-tutorial.md`.
