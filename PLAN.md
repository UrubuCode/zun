# Zun — Plano de Arquitetura

Máquina de linguagem JS/TS em Rust, inspirada na arquitetura do .NET (CLR/CoreCLR), usando **Cranelift** como backend de código nativo. Não é "mais um runtime JS estilo V8": é uma plataforma com **IL própria, metadata, compile e runtime separados**, onde JS/TS é o front-end — como C# é para o .NET.

Base de referência: `.docs-dotnet/` (análise do dotnet/runtime, 10 áreas). As decisões abaixo seguem as recomendações desses documentos.

---

## Implementation status update (2026-10-20)

User-module execution now compiles source module sets, emits a manifest,
validates `MemberRef` across loaded scopes, and executes external calls in Tier
0. The interpreter also uses compact inherited virtual-slot tables instead of
a type-by-all-methods matrix.

Dynamic override dispatch in the JIT is implemented through compact inherited
virtual-slot tables. Precise interpreter roots, native/JIT stack-map metadata,
Promise/microtask primitives (including settlement-preserving `finally`), `Any`/`DynamicObject`, scalar external-library
calls, interpreted backedge observation, and conservative reachability-based
trimming are also implemented. Scalar typed FFI/ABI integration now covers
source declarations, typed MemberRef metadata, host resolution, and direct
blittable dispatch. The remaining gates are non-blittable marshalling,
production N-API hosting, full-runtime AOT/R2R object emission, true OSR frame replacement,
and production-grade
trimming diagnostics and metadata removal.

The JIT virtual-call path is covered by interpreter/JIT equivalence, including
derived overrides. The remaining object-runtime gate is precise root handling
at collection safepoints; virtual dispatch itself is no longer Tier 0-only.

Language surface since then: exceptions (`throw`/`try`/`catch`/`finally`, with
interpreter unwind, rethrow routing to outer handlers in the same frame, and an
explicit Tier 0 fallback for methods with handler regions in the JIT) are
complete; anonymous and arrow functions include closure HIR/sema/ilgen,
binding captures, mutable-cell boxing, and GC tracing. The ordered remaining backlog and the
rules that govern it are the active execution goal in section 6 (mirrored by
`GOAL.md`).

## 1. Visão geral do pipeline

```
 fonte .ts/.js
     │
     ▼
 zun_frontend (parser + binder + type checker TS)
     │  AST tipada
     ▼
 zun_ilgen (lowering: closures, async state machines, pattern/decision trees)
     │
     ▼
 ZIL (Zun IL — bytecode de pilha + metadata em tabelas)   ← formato em zun_il / zun_metadata
     │  empacotado em módulo .zm (container mmap-ável)
     ▼
 zun_vm (loader, type system reificado, verificação)
     ├── Tier 0: interpretador (zun_interp)
     └── Tier 1: JIT Cranelift (zun_codegen) — tiering + OSR depois
          └── futuramente: AOT (cache estilo R2R sobre a mesma ZIL)
```

Princípio central (doc 02): **compile e runtime são produtos separados que se comunicam por um formato binário estável (ZIL + metadata)**, nunca por AST em memória. Isso dá cache de compilação, distribuição de módulos pré-compilados, verificação independente e AOT no futuro.

## 2. Organização em crates (workspace Cargo)

```
zun/
├── Cargo.toml                 # workspace
├── crates/
│   ├── zun_common/            # tipos básicos compartilhados: interning, arenas, ids, diagnostics
│   │
│   │   # ---- formato (o "ECMA-335" do projeto) ----
│   │   ├── zun_il/            # definição da ZIL: tabela mestre de opcodes (gera asm/disasm/
│   │   │                      #   verificador via macro, estilo opcode.def), tipos de pilha
│   │   ├── zun_metadata/      # tabelas (TypeDef/MethodDef/TypeRef/MemberRef...), tokens
│   │   │                      #   (tabela<<24|rid), heaps dedup (strings/blobs), assinaturas
│   │   │                      #   prefixadas; leitor mmap read-only + builder editável
│   │   └── zun_module/        # container .zm: header versionado, RVAs, corpo tiny/fat,
│   │                          #   build determinístico, debug info (sequence points)
│   │
│   │   # ---- compile-time (front-end TS/JS) ----
│   │   ├── zun_syntax/        # lexer + parser TS/JS → AST (com spans)
│   │   ├── zun_sema/          # binder, escopos, type checker TS, nullability,
│   │   │                      #   definite assignment, inferência p/ JS
│   │   ├── zun_ilgen/         # lowering AST→ZIL: closures→display struct (env,code),
│   │   │                      #   async/generators→state machines stackless,
│   │   │                      #   switch/match→árvore de decisão, EH→tabela de regiões
│   │   └── zun_compiler/      # driver: resolução de módulos em build, manifesto do grafo
│   │                          #   achatado (estilo deps.json), emissão de .zm
│   │
│   │   # ---- runtime (a VM) ----
│   │   ├── zun_types/         # type system reificado em runtime: descritores de tipo
│   │   │                      #   (MethodTable), vtables lineares, interface maps,
│   │   │                      #   generics (tabela (def,[args])→tipo), layout de objeto
│   │   ├── zun_verify/        # verificador de ZIL (type safety, pilha consistente em joins)
│   │   ├── zun_loader/        # ModuleScope (≈AssemblyLoadContext): identidade (nome,escopo),
│   │   │                      #   ordem de resolução fixa (cache→hook→pai→resolving→erro),
│   │   │                      #   leitura do manifesto; sem scan de diretório
│   │   ├── zun_gc/            # GC geracional móvel: TLAB bump alloc, card table/write
│   │   │                      #   barrier, mark-sweep-compact (plan separado), LOH, handles
│   │   │                      #   (strong/weak/pinned), interface estreita VM↔GC
│   │   ├── zun_interp/        # Tier 0: interpretador da ZIL (correto primeiro, rápido depois)
│   │   ├── zun_codegen/       # Tier 1: ZIL→Cranelift IR. Interface estreita com a VM
│   │   │                      #   (trait estilo ICorJitInfo), entry points redirecionáveis
│   │   │                      #   (precode/stub com troca atômica) p/ tiering futuro
│   │   └── zun_vm/            # orquestra tudo: engine, threads, exceções (2 passadas,
│   │                          #   tabela de regiões), scheduler de tasks/futuros, intrinsics
│   │
│   │   # ---- biblioteca padrão ----
│   │   ├── zun_core/          # o "System.Private.CoreLib": intrinsics declarados + layouts
│   │   │                      #   congelados de Object/String/Array/Function, boxing
│   │   └── std/               # std escrito em TS (!): String, Number, Array, Map, Math,
│   │                          #   Promise... compilado para .zm pelo próprio compiler;
│   │                          #   métodos "mágicos" marcados @intrinsic com fallback em TS
│   │
│   │   # ---- hosts ----
│   │   ├── zun_cli/           # binário `zun`: build, run, disasm, verify
│   │   └── zun_napi/          # bindings N-API para hospedar a VM dentro do Node (fase 2)
│   └── ...
└── .docs-dotnet/              # referência de design
```

Regra de dependência: `formato ← compile-time` e `formato ← runtime`, mas **compile-time e runtime não dependem um do outro** (exceto o driver/CLI que usa os dois). `zun_il`/`zun_metadata` são o contrato.

## 3. Decisões de design (mapeadas aos docs)

### IL e metadata (doc 02)
- ZIL = bytecode **de pilha**, opcodes de 1 byte + prefixo de escape, definidos em **tabela mestre única** (macro Rust) que gera assembler, disassembler, verificador e importador do codegen.
- Instruções de alto nível **usam tokens, nunca offsets** (`newobj <token>`, `callvirt <token>`, `ldfld <token>`) — layout de objeto fica livre para evoluir.
- Metadata em tabelas com heaps deduplicados; assinaturas com gramática prefixada e inteiros compactos.
- Debug: sequence points por offset de IL, num formato tipo Portable PDB embutido/sidecar.

### Objeto e valores JS — substituto do "hidden box" (docs 03, 04, 05)
Em vez de NaN-boxing/hidden classes do mundo V8, o modelo é o do CLR:
- **Objeto = [ponteiro de tipo][header multiplexado (hash/flags)]** + campos com layout fixo decidido pelo compilador a partir dos tipos TS.
- Código **tipado (TS)** usa campos diretos e dispatch por vtable/interface map — sem shape checks.
- Código **dinâmico (JS puro / `any`)** usa um tipo `DynamicObject` do core (dicionário) + um valor `Any` (tagged union: smi, f64, ref) com `box`/`unbox` explícitos na IL, elimináveis por análise de escape. Ou seja: o caso dinâmico é a exceção que paga o custo, não o modelo base — o inverso do V8.
- Vtable linear com slots mutáveis (viabiliza JIT tardio/tiering); dispatch de interface separado com inline cache monomórfico → cache global → itable.
- Generics do TS **reificados** (tabela `(def,[args])→tipo`), começando sem compartilhamento de código; `__Canon`-sharing só quando medirmos inchaço.

### GC (docs 05, 09)
- Geracional móvel, 2 gerações, workstation/stop-the-world primeiro, estruturas já separadas em "por heap" vs globais.
- TLAB bump allocation (~8 KB), write barrier como instrução distinta na IL, card table, LOH (~85 KB), heap congelado para literais/std pré-inicializado.
- Interface estreita VM↔GC desde o dia 1 para poder trocar a implementação.

### Execução (doc 09)
- **Tier 0 interpretador** → **Tier 1 Cranelift** método a método, atrás de um trait estilo `ICorJitInfo` (resolver tokens, layouts, helpers de alocação/barrier). Entry points redirecionáveis com patch atômico desde o início — tiering e OSR entram depois sem redesign.
- AOT/cache estilo R2R fica para fase posterior, mas o formato .zm já reserva header de extensão para isso.

### Controle de fluxo e async (docs 06, 10)
- Exceções por tabela de regiões (custo zero), duas passadas.
- `async/await` e generators viram **state machines stackless no ilgen**; Promise = futuro do runtime com estado atômico CAS e campo único de continuação; caminho síncrono sem alocação.
- Event loop/microtasks implementados sobre o scheduler de futuros (necessário para semântica JS).

### Módulos (doc 01)
- Resolução de import acontece **no build**: o compiler gera manifesto com grafo achatado; o loader do runtime só segue o manifesto (cache → hook → escopo pai → erro com tentativas).
- `ModuleScope` como objeto explícito; identidade de tipo inclui o escopo.

### std em TS (doc "CoreLib")
- `zun_core` (Rust) define só o mínimo: layouts congelados, alocadores, intrinsics (ex.: `Math.sqrt`, operações de string UTF-16, `Array` interno).
- Todo o resto do std é **TS tipado** compilado pelo próprio compiler — dogfooding do pipeline desde cedo. Intrinsics via decorator `@intrinsic("...")` com corpo de fallback em TS.

## 4. Fases de implementação

**Fase 0 — Fundação do formato**
Workspace + `zun_common`, `zun_il` (tabela de opcodes + asm/disasm), `zun_metadata`, `zun_module`. Testes round-trip (emitir → ler → disasm).

**Fase 1 — Pipeline mínimo ponta a ponta**
Parser/sema para um subconjunto TS (funções, números, strings, if/while, objetos com classe), ilgen, verificador, loader, `zun_types`, **interpretador** rodando `.zm`. GC pode ser arena/leak nessa fase. Marco: `zun run hello.ts`.

**Fase 2 — Runtime de verdade**
GC geracional + barriers, exceções, closures, vtables/interfaces, generics reificados, começo do std em TS com intrinsics. Marco: benchmarks simples e std de String/Array/Map funcionais.

**Fase 3 — Cranelift Tier 1**
`zun_codegen` atrás do trait tipo ICorJitInfo, entry points com patch atômico, tiering por contador de chamadas. Marco: paridade de corretude interp↔JIT (suíte de equivalência rodando nos dois modos, como o doc 02 recomenda).

**Fase 4 — Async, dinâmico e JS**
State machines async, Promise/event loop, `Any`/`DynamicObject` para JS não tipado, inline caches no dispatch.

**Fase 5 — Hosting e distribuição**
`zun_napi` (Node embedding), módulos pré-compilados/cache R2R-like, OSR, trimming/alcançabilidade.

### Estado de implementação

As fases não são apenas marcos históricos: cada item abaixo precisa ter um
contrato observável no código e um teste que o exercite.

- **Entregue:** formato `.zm` com round-trip determinístico, parser/sema/ilgen,
  verificador, loader, interpretador, GC mark-sweep, layouts compartilhados,
  intrinsics indexados, módulos de código Zun (`std/math` e `std/string`),
  manifesto binário e geração do manifesto pelo compiler.
- **Entregue parcialmente:** Tier 1 Cranelift para valores numéricos, booleanos
  e objetos com fields numéricos; equivalência interp↔JIT coberta por testes.
  Um caminho estreito para arrays de objetos também baixa leituras e escritas
  de ponteiros gerenciados, com equivalência coberta no JIT; casts herdados e
  o portador agregado `Any` continuam deliberadamente no Tier 0.
  Intrinsics numÃ©ricos de `zun:date` e `zun:math` usam uma ABI C estÃ¡vel e
  lowering direto para imports do JIT. Chamadas virtuais com override ainda
  ficam no Tier 0.
- **Entregue parcialmente:** resolução de exports e `MemberRef` entre escopos
  carregados. Imports de módulos de usuário são validados no build, mas ainda
  não são baixados para uma chamada executável no VM.
- **Ainda aberto:** otimização Cranelift de generators/async e closures,
  ABI/FFI tipado integrado à IL, bindings Node N-API de produção, AOT/R2R
  completo para métodos fora do subconjunto escalar, substituição de frames
  por OSR e trimming completo de metadata/diagnósticos. A declaração
  `function*`, o lowering de `yield`, a retomada persistente no Tier 0 e a
  integração dos generators com `next()`/`for...of` já estão implementados e
  testados; somente o suporte Tier 1 permanece pendente.

## 4.1 Modos de distribuição (decisão: suportar os três)

O usuário final nunca instala um runtime. Os três modos são empacotamentos do mesmo pipeline — o codegen atrás do trait estilo `ICorJitInfo` serve JIT (`cranelift-jit`) e AOT (`cranelift-object`) sem saber a diferença:

1. **Self-contained JIT (default do `zun build`)** — executável único: apphost nativo + VM linkada estaticamente + `.zm` embutido como seção de dados. Duplo clique e roda; JIT em runtime. Equivale ao single-file self-contained do .NET.
2. **Self-contained + R2R** — modo 1 com código nativo pré-compilado embutido (via `ObjectModule`) e ZIL como fallback para o JIT. Startup quase instantâneo, mantém dinamicidade.
3. **AOT puro (`zun build --aot`)** — Cranelift roda inteiro no build, linker do sistema gera o binário; runtime mínima embutida (GC, scheduler, exceções, estilo Go), mundo fechado com trimming e reflection reduzida (docs 08 e 02-compilation/07).

Restrições de plataforma que tornam o modo 3 obrigatório: **iOS e consoles proíbem JIT** (sem páginas executáveis); Android aceita JIT mas AOT dá melhor startup. Desktop (Win/Linux/macOS) pode usar qualquer modo. Ordem de entrega: 1 → 2 → 3, conforme doc 02-compilation.

## 4.2 Tipos numéricos e a pilha de avaliação tipada (decisão)

O primeiro corte do JIT (Fase 3) representa **todo valor como f64** (booleanos como 1.0/0.0). Isso é **andaime**, não o modelo final — é fiel ao `number` de JS (que é f64) e evita rastrear tipos na pilha no começo, mas não escala: índices, deslocamento de bits, `| 0`, referências de objeto e strings não são f64.

**Modelo final (estilo .NET / ECMA-335):** a pilha de avaliação do JIT rastreia o **tipo Cranelift de cada valor** — `I64` para refs/bools, `I32`/`I64` para inteiros, `F64` para float — derivado da HIR tipada e das assinaturas, e emite a instrução de máquina certa (ALU inteira vs. XMM). Opcodes já são separados por família na tabela mestre (`add.i32` vs `add.f64`), seguindo o doc 02.

**Extensão do TS com tipos numéricos explícitos (decisão do usuário):** a linguagem Zun vai expor tipos numéricos distintos no TS, além do `number` do JS:

```ts
const valor: int = 1;      // inteiro de máquina (i32/i64)
const test: float = 1.0;   // ponto flutuante (f64)
```

- `number` (JS) continua existindo e mapeia para `float` (f64), por compatibilidade com código JS.
- `int` (e futuramente larguras como `i32`/`i64`/`u32`) dá ao compilador o tipo estático exato, então o JIT usa registradores inteiros e operações inteiras sem conversões `ToInt32` em cada passo — a complexidade que o f64 uniforme empurraria para o runtime desaparece no compile-time.
- O JS puro (sem anotação) permanece `float`/`Any`; o TS tipado ganha a capacidade de ser preciso. Isso é coerente com o princípio do projeto: "TS sound por padrão, dinâmico como opt-in custoso".

### Reticulado numérico completo

`int`/`float` são o começo; o alvo é a família completa de larguras fixas, como em Rust/.NET. Usuários comuns usam `int`/`float`; usuários que precisam de controle de bits (e o próprio `std`) usam as larguras exatas.

| Tipo TS | Repr. de máquina | Cranelift | Uso típico |
|---|---|---|---|
| `float` / `number` (JS) | f64 | `F64` | número geral, compat. JS |
| `f32` | f32 | `F32` | gráficos, SIMD, dados compactos |
| `int` | i32 (padrão) | `I32` | contadores, índices |
| `i8` `i16` `i32` `i64` | inteiro com sinal | `I8..I64` | valores com sinal de largura exata |
| `u8` `u16` `u32` `u64` | inteiro sem sinal | `I8..I64` (sinal no opcode) | bytes, buffers, UTF-16, flags, hashing |
| `bool` | i8 (0/1) | `I8` | condições (hoje já é `Boolean`) |

Decisões de semântica:
- **Sinal fica na operação, não no tipo Cranelift** (como no próprio Cranelift e no doc 02): o reticulado guarda a assinatura; `div`/`shr`/comparações escolhem a forma com/sem sinal (`sdiv`/`udiv`, `sshr`/`ushr`, `IntCC::Signed*`/`Unsigned*`).
- **Overflow explícito por operação** (doc 08/10): `wrap` (padrão, como `u8` do Rust em release — útil para hashing/buffers), com modo `checked` (trap) opcional. A tabela mestre já prevê escolher wrap/trap por instrução.
- **Conversões explícitas** entre larguras (`as`), lowering para `ireduce`/`uextend`/`sextend`; number↔int usa os `fcvt_*_sat` (nunca a forma que faz trap — o pitfall do guia do Cranelift).
- **Representação no interpretador**: hoje `Value::Number(f64)`; o plano é `Value::Int(i64, width, signed)` + `Value::Float(f64|f32)` ou variantes dedicadas por largura. O enum absorve isso sem mudança estrutural.
- **Layout de objeto**: larguras exatas permitem campos compactos (`u8`/`u16`) com o alinhamento mínimo (doc 03/05), importante para structs densos e interop.

### Ordem de implementação

1. `zun_sema`: substituir `Ty::Number` por um reticulado (`Ty::Float{32,64}`, `Ty::Int{width, signed}`), com `number`→`float64` e regras de promoção/conversão.
2. `zun_ilgen`: escolher a família de opcode certa (`add.i32`/`add.f64`/…) e inserir conversões; emitir larguras estreitas com máscara/extensão.
3. `zun_interp`: variantes de `Value` por largura (ou i64+largura), com wrap correto.
4. JIT: pilha tipada (I8/I16/I32/I64/F32/F64) em vez de "tudo f64", com sinal/overflow na instrução.
5. `std` base: usar `u8`/`u16`/`u32` em buffers, código de string (UTF-16), hashing e flags.

## 4.3 GC e objetos no JIT (plano — o maior item restante)

Hoje o JIT compila o subconjunto numérico/bool e a leitura sem alocação do
comprimento de strings internadas; métodos que tocam o heap
(string, objeto, `ldfld`/`stfld`/`newobj`/`callvirt`) caem no interpretador,
que usa `Rc<RefCell<Vec<Value>>>` para objetos. Para levar objetos ao JIT,
interpretador e JIT precisam compartilhar um **heap de ponteiros crus**. Ordem
de implementação:

1. **Tipos de campo no formato** (pré-requisito, baixo risco): gravar o tipo de
   cada `FieldDef` (um `JitKind`/código de tipo) no `.zm`, mais o tamanho e os
   offsets do objeto por `TypeDef`. Sem isso o JIT não sabe o layout. *(Primeiro
   passo sendo implementado agora.)*
2. **Heap de runtime não-móvel**: um alocador (`zun_gc`) chamável por
   `extern "C"` do código nativo e direto do interpretador. Objeto = bloco
   `[type_rid][campos...]`, campos em slots crus interpretados pelo tipo
   estático. Sem coleta ainda (bump/leak) — "alocar primeiro, coletar depois"
   (doc 05/09).
3. **Unificar o modelo de objeto**: reescrever o interpretador de `Rc` para
   ponteiros crus no mesmo heap, para que objetos atravessem a fronteira
   interp↔JIT. Código `unsafe` isolado atrás de uma API segura.
4. **Objetos no JIT**: `newobj` chama o alocador; `ldfld`/`stfld` viram
   load/store por offset; `callvirt` indexa a vtable (as faixas
   `method_start/method_end` já existem) — slot em vez de busca por nome.
5. **GC real**: mark-sweep sobre o heap não-móvel. *(Coletor implementado e
   testado — `Heap::collect` marca a partir de um conjunto de raízes, traça os
   campos-referência por tipo e varre os inalcançáveis para uma free-list.)* O
   que falta é o **disparo automático ao vivo com raízes precisas**: safepoints
   onde o interpretador expõe os `Value`-objeto das pilhas/locais de todos os
   frames, e as raízes das pilhas nativas via stack maps do Cranelift (guia
   §21). Compactação (mover objetos) fica para depois, pois exige reescrever
   referências e stack maps mais ricos.

Estado: alocação, layout, acesso a fields, comprimento de strings internadas e
coleta mark-sweep estão implementados
e testados. O JIT já suporta `newobj`, `ldfld` e `stfld` para campos numéricos e
mantém equivalência com o interpretador. Resultados de `newobj` são marcados como
referências gerenciadas nos mapas precisos do Cranelift e foram cobertos enquanto
o objeto permanece vivo através de uma chamada JIT. `callvirt` geral ainda é executado no Tier 0;
`String.length()` é uma exceção direta por handle internado;
o JIT já possui dispatch virtual por slot para os métodos elegíveis sem override;
dispatch dinâmico completo e chamadas virtuais com fallback continuam sendo
fronteiras separadas, preservando a resolução de tokens e a tabela de métodos.

Depois do dispatch virtual, a integração operacional de raízes precisas está
implementada: safepoints automáticos no interpretador/JIT capturam `(PC, SP)`
na slow path de alocação, publicam o frame parado e consomem a metadata de
slots do Cranelift. A coleta/compactação reescreve os slots registrados, com
equivalência e testes de coleta durante chamadas JIT. Permanecem apenas
extensões de cobertura multiplataforma e a coordenação multi-thread, que está
explicitamente deferida até o VM ter múltiplos mutators.

O benchmark de Tier 0 também cobre loops de leitura/escrita de campos e dispatch
virtual. Os lookups de metadata desses caminhos são pré-resolvidos na carga do
módulo para indexação O(1). O próximo trabalho de performance é medir e reduzir
o custo de execução de chamadas virtuais no JIT, sem misturar isso com OSR ou a
integração de stack maps.

## 4.4 Biblioteca padrão, intrinsics e FFI (design)

A stdlib do Zun tem dois tipos de módulo, mais um mecanismo externo futuro —
separados de propósito, como Rust (`std` + FFI) e .NET (BCL + InternalCall vs
P/Invoke):

- **Módulos de código Zun** (ex.: `zun:std` — matemática): `.ts`/`.js`/`.zun`
  puros, compilados como código de usuário. Para lógica que não toca o SO.
- **Intrinsics** (ex.: `zun:fs`, `zun:io`, `zun:env`, `zun:time`): assinatura
  declarada em Zun, corpo implementado **dentro do runtime** (em Rust). É o
  análogo do **InternalCall/FCall** do .NET e dos intrinsics/`std::sys` do Rust.
  Chamados por um dispatch indexado O(1) (`call.builtin <id>`); cada intrinsic
  tem um id fixo que o compilador resolve a partir do `import`. No JIT viram
  `extern "C"` diretos no futuro.
- **FFI / P-Invoke** (futuro, **mecanismo separado**): carregar bibliotecas
  nativas **externas** (`.dll`/`.so`/`.dylib`) em runtime, com marshalling de
  tipos (doc 07-actions-operations/04). É o `[DllImport]`/`LibraryImport` do
  .NET e o `extern "C"` sobre libs do Rust. NÃO é o mesmo que intrinsic: o
  intrinsic é implementado por nós; a FFI chama código de terceiros.

  Non-blittable string marshalling is explicitly deferred and must declare
  ownership in the signature. Zun strings are converted between internal
  UTF-16 and UTF-8 at the boundary. Inputs use call-scoped native temporary
  storage and pass either `(pointer, length)` or a declared NUL-terminated
  form. Returns must explicitly declare borrowed data with a length source,
  `owned(free_fn)`, or `out_buffer(capacity)` (preferred where supported).
  No return ownership mode is inferred; every native pointer is copied before
  the call boundary ends.

Resolução de módulo: o prefixo `zun:` marca módulos embutidos. `import { readText
} from "zun:fs"` → o compilador procura o módulo no registro de intrinsics,
vincula os nomes com suas assinaturas e baixa chamadas para `call.builtin <id>`.
Módulos de usuário (caminhos relativos) e o sistema de módulos de código Zun
ficam para uma fase posterior (precisam de compilação multi-arquivo).

### 4.4.1 Collections and identity prerequisites

Strong `Map` and `Set` must use runtime-owned hash tables with insertion-order
storage; they must not be linear scans over `Array`. Primitive keys use value
hashing, while object keys use a stable identity hash. The shared object layout
reserves a header slot for that hash, and the non-moving heap assigns a fresh
hash when an allocation is created or a reclaimed block is reused. This keeps
object-key hashing valid when a future compacting collector changes addresses.
The ordered table uses stable entry indices and tombstones, so deletion does
not shift the insertion-order vector or become an O(n) operation. Automatic
compaction bounds tombstone memory after repeated deletion. Heap APIs expose
Map entries and Set values in insertion order, plus `has` and `clear`
operations for the strong collection runtime contract.

Weak collections are a separate GC phase: weak slots require post-mark
clearing, `WeakMap` requires ephemeron fixed-point marking, and
`FinalizationRegistry` requires a finalization queue. None of these APIs may be
represented by a strong array wrapper that silently violates reachability
semantics.

### 4.4.2 JavaScript/TypeScript language-feature roadmap

Beyond the built-in classes, the language surface itself is still incomplete.
Features are grouped by how much runtime/compiler architecture they force,
mirroring the way weak collections forced dedicated GC work.

**Group 1 — features that force runtime/compiler architecture:**

- First-class functions and closures (anonymous `function` expressions, arrow
  `=>`). A function value is a `(code pointer, captured environment)` pair; the
  environment is heap-allocated and GC-traced (the .NET display-class model).
  This is what makes functional `Array.map`/`filter`/`reduce` and callbacks
  possible at all. *Status: Tier 0 implementation complete — closure
  construction, binding captures, boxed mutable locals, nested environments,
  callback lowering, and interpreter GC tracing are covered by tests. Broader
  Cranelift closure-call lowering remains an optimization gate.*
- Exceptions: `throw`/`try`/`catch`/`finally`. Requires IL handler regions,
  verifier support, interpreter unwinding, and Cranelift landing pads. The
  `Error` hierarchy already exists but is unusable without throw/catch control
  flow. *Status: Tier 0 complete — handler regions, verifier checks, nested
  unwind, cross-frame propagation, and finally-on-return/throw are covered;
  Cranelift methods with exception regions explicitly fall back to the
  interpreter.*
- Iteration protocol (`for...of`, `Symbol.iterator`) and generators
  (`function*`). Generators lower to a state machine, reusing the async
  state-machine substrate. *Status: `SymbolRegistry`, `for...of` for Array,
  Map, and Set, source-level iterator calls, generator declaration syntax,
  state-machine planning/lowering, and persistent Tier 0 resume are implemented;
  broader Cranelift generator support remains an optimization gate.*

**Group 2 — syntactic / medium features (no deep runtime change):** spread/rest
(`...`), array and object destructuring, default and rest parameters, optional
chaining `?.` and nullish coalescing `??`, template literals, `switch`,
`do/while` and C-style `for(;;)`, labelled `break`/`continue`, ternary and
short-circuit `&&`/`||` value semantics, getters/setters, `static`
fields/methods, and private `#fields`. *Status: `switch`, `do/while`, C-style
`for(;;)`, labelled control flow, short-circuit `&&`/`||`, ternary conditional
expressions, explicit `null`, nullish coalescing `??`, optional property and
method chaining, trailing default parameters, spread/rest, destructuring, and
template literals and static class methods are implemented with complete
parser, sema, metadata, verifier, and lowering paths. Static fields now have
complete source-to-IL Tier 0 read/write lowering and GC-rooted storage; the JIT
uses its existing interpreter fallback for these field operations. Getters/setters
      are implemented as `get`/`set` accessor sugar over ordinary methods;
      and private `#fields` are implemented; private members are mangled in
      metadata/layout and access is checked against the declaring type.*

**Group 3 — TypeScript type-level (erased at runtime):** generics, interfaces,
enums, union types with narrowing, and optional properties. These are handled by
the checker and emit no new code except `enum`. *Status: separate checker
track.*

Managed `Map`/`Set`/`WeakMap` constructors and receiver bindings are now
available from source as thin classes over the runtime floors, and `for...of`
covers Array plus insertion-ordered Map/Set values through the interpreter
fallback. Source-level `Symbol.iterator` calls now resolve to canonical
iterator handles, and typed-array/DataView source constructors are available
through the interpreter path. `DataView` is integrated with the managed
buffers; `RegExp` now has a managed Rust regex floor and a Tier 0 source
wrapper (Cranelift remains explicitly deferred); `Reflect` now has Tier 0
`get`/`set`/`has`/`delete` operations over `DynamicObject`, including runtime
property keys through dedicated IL, while `Proxy`
traps remain on the dynamic `Any`/`DynamicObject` path (Phase 4); `Proxy`
construction now binds to those strongly traced target/handler records, and
dynamic handlers can retain named closure traps. `dynamicSetFunction` exposes
that callable-property binding to source code, and `proxyGet`, `proxySet`,
`proxyHas`, and `proxyDelete` invoke named `get`/`set`/`has`/`deleteProperty`
traps or fall back to the target's dynamic property in Tier 0; remaining Proxy
trap forms and call-site caching are still Phase 4 work. `BigInt`
needs a bignum
(deferred); `Intl`, `SharedArrayBuffer`, and `Atomics` are out of scope for now.

Cross-cutting rule: data structures with real algorithmic complexity (hashing,
buffers) live in the Rust intrinsic floor; the TypeScript class is only a thin
typed wrapper over that floor — never TypeScript-over-`Array`.

## 4.5 Dynamic path: unified object model, shapes, and call sites (design)

**Validation.** For a typed VM whose dynamic code is the exception, the proven
design is the .NET **DLR** (doc 07/09): per-call-site caching of `(guard, action)`
rules, combined with **shapes** (hidden classes) for dynamic objects. This beats
V8/JSC-style whole-method speculation + deopt *for Zun*, because the fallback is
local to each site (no frame reconstruction), and it stays strictly scoped to the
`any`/dynamic path — typed code keeps the CLR fixed-layout model of section 3
untouched. Alternatives considered and rejected: always-dictionary objects (too
slow); whole-program type inference (that is exactly what TS types already give
the typed subset); tracing JITs (more complex, no better fit here). This section
refines section 3's "`DynamicObject` (dicionário)" into shape-based storage with
a dictionary only as the fallback mode.

### Unified object model

One object shape for both worlds, so the fast access path is identical:

- header = `[shape/class ptr][multiplexed hash/flags]` + a flat slot array.
- A **typed class** is a shape *frozen at compile time* from TS types (section 3):
  field access is `load [slots + offset]` with **no guard**, because the static
  type proves the shape.
- A **dynamic object** gets a **runtime shape**: adding a property transitions
  along a shape-transition tree; objects with the same keys+order share a shape
  and the same flat slots, so access is `load [slots + offset]` behind a cheap
  `shape == S` guard. A pathological / megamorphic object degrades to a
  dictionary (today's `DynamicObject` map) as the fallback — like V8 dictionary
  elements. This replaces the always-`BTreeMap` `DynamicObject` with shape+slots,
  dictionary only as fallback.

This is the analogue of .NET's `ExpandoClass` behind `ExpandoObject` (doc 07/09),
and it is the "ideal middle" between a fully static class and a fully dynamic
dictionary: the same machine-level access for both, with a guard only where the
shape was discovered at runtime.

### Call sites (DLR-style)

Each dynamic operation (`GetMember`, `SetMember`, `InvokeMember`, `BinaryOp`,
index, convert) is a call site caching `(guard, action)` rules: L0 monomorphic
on the site, L1 a small per-site polymorphic set (~10), L2 shared per binder
(~128); on total miss the binder re-resolves and installs a new rule, and a
megamorphic site stays on the slow (cold) binder path. The guard
(`tag ==` / `shape ==`) plus the **local** fallback are the correctness core —
there is no whole-method deopt. The rule action is an interpreter closure in
Tier 0 and a compiled stub in Tier 1, reusing the existing interpreter/JIT rather
than a second codegen.

### Boxing (the ideal middle)

Typed values are already unboxed (native i32/f64/object-ptr on the typed stack);
only `any` is boxed. Keep `Any` as a compact tagged value for the dynamic path and
keep its representation behind an abstraction so it can change. **Defer**
NaN-boxing / pointer-tagging: their main win (one word per value) matters when
*every* value is dynamic (the V8/JSC world); here typed values never box, so the
pressure is low. Revisit NaN-boxing only if a dynamic-heavy workload measures it
as worth the complexity. Rule: never box what is typed; box only `any`, compactly.

### Array and Object in the VM

Storage with real complexity stays in the Rust floor (as `ArrayStorage` already
does); the `.zun` class is a thin surface. A generic / `any` `Array` stores boxed
`Any` elements over that floor; typed arrays keep their primitive buffers.
Optional later: V8-style elements-kind specialization (packed int/double/holey/
dictionary) if mixed arrays prove hot — performance backlog, not now.

The generic `Any` carrier ABI is intentionally deferred in Cranelift. Dynamic
operations such as `Map.has(any)` and `Set.has(any)` remain Tier 0 unless a
measured workload justifies a dedicated lowering; typed keys and values use the
existing scalar JIT ABIs. This is a design boundary for the opt-in dynamic
path, not a correctness gap.

### Payoff beyond property access

Shapes make `JSON.parse` of uniform records (the common case — arrays of
same-shaped objects) converge to a shared shape, giving compact storage and fast
field access; `JSON.stringify`, serialization, and reflection iterate shapes. So
the same mechanism pays for JSON, serialization, and reflection, not only `o.x`.

Grounded in docs 07/09 (DLR) and 03/04/05 (object layout, dispatch, allocation).
Scope: the `any` / `DynamicObject` / `Proxy` / `Reflect` path (section 4.4.2);
typed code is unaffected.

### Validated reuse opportunities (benchmarked 2026-10-20)

The same "resolve-once + cache + indexed access" principle was benchmarked in an
isolated worktree in the places it was proposed (ns/op; `x` = how many times
faster the indexed/cached form is). All confirmed:

| Case | lookup / linear | cached / indexed | x |
|---|---|---|---|
| Dynamic object property read (BTreeMap vs shape+IC) | 25.9 ns | 3.0 ns | 8.6 |
| Dynamic object property write | 104.5 ns | 2.3 ns | 45 |
| Interface dispatch, monomorphic (selector lookup vs inline cache) | 7.6 ns | 2.0 ns | 3.8 |
| `switch` over 32 cases (if-chain vs jump table) | 5.6 ns | 0.5 ns | 11.3 |
| IC guard (string-eq vs interned-id-eq) | 4.6 ns | 0.55 ns | 8.3 |
| Symbol-keyed property read (`BTreeMap<id>` vs flat slot) | 20.1 ns | 1.2 ns | 16.5 |

Consequences for the design:

- **Interface dispatch**: build **one** polymorphic inline-cache primitive and
  reuse it for both dynamic member operations (this section) and interface calls
  (section 3). Caveat from the numbers: a precomputed **slot-indexed** vtable is
  already ~as fast as the IC for the monomorphic case, so prefer a slot when the
  concrete type resolves to one and use the IC for the interface→impl resolution
  that a slot cannot express directly. The 3.8x is over selector-by-name lookup.
- **Guards must use interned name ids** (`u32`), not strings — ~8x cheaper; use
  `zun_common`'s interner as the substrate for both shapes and ICs.
- **Symbol-keyed properties** (`DynamicObject.symbol_properties` /
  `symbol_functions`, today `BTreeMap`) must live in the shape/slot model too.
- **`switch`** over many/dense cases should lower to a jump table in ilgen/codegen.

## 4.6 Front-end: native TS/JS via OXC (decision, 2026-10-21)

Goal: Zun targets **native TS/JS** to compete with Bun/Node, not only its own
`.zun` dialect. JS/TS is a *borrowed* front-end (as C# is to .NET), so Zun does
**not** own the grammar and will not hand-write the full TS/JS parser; it adopts
a mature Rust parser as the front-end feeding the existing sema/HIR/ilgen. The
ZIL-only compiler↔VM invariant is unaffected — the AST is internal either way.

**Choice: OXC (`oxc_parser`), not SWC.** Benchmarked in isolated scratch crates
(2026-10-21): OXC is lighter on every axis (build time a tie):

| metric | OXC `oxc_parser` 0.155 | SWC `swc_ecma_parser` 46 |
|---|---|---|
| transitive crates | 105 | 122 |
| compiled `.rlib`s | 71 | 85 |
| clean release build | 42 s | 40 s |
| `target/release/deps` | 215 MB | 267 MB |

OXC is also library-first (parser + arena allocator, built to embed), has a
cleaner AST than SWC (whose AST is shaped around its own transforms), and parses
faster in published benchmarks. Caveat: both are **heavy** dependencies (100+
crates, 200+ MB artifacts) — that is the real cost of a full TS/JS parser; OXC
merely minimizes it.

Plan:

- Keep the hand-written `zun_syntax` parser for now (the team is mid-runtime);
  do not swap mid-flight.
- When broad TS/JS support is the active target, add `oxc_parser` as the
  front-end and write an **adapter `oxc_ast` → Zun HIR** (sema/HIR/ilgen stay).
  The `.zun` dialect and the hand parser coexist during the transition.
- Express Zun's non-standard syntax as **valid TS** so a stock parser accepts it:
  intrinsic/`extern` bodies as `declare` methods + a decorator (`@intrinsic`),
  machine-width types (`u8`..`u64`, `i8`..`i64`, `f32`) as ordinary type
  references resolved by sema. Do not invent keywords the parser would reject.
- Before committing, prototype the `oxc_ast` → HIR adapter for one construct in a
  worktree and measure the adapter effort (per the "measure before asserting"
  rule).

## 5. Implementation ledger (2026-10-20)

### Current ABI gates (2026-10-20)

- `Any` has a stable two-word ABI contract (`tag`, `payload`) with validated
  reconstruction and layout tests in `zun_core`.
- The JIT runtime now exposes a non-allocating, out-parameter helper for
  copying an array element into that two-word carrier; this establishes the
  ABI boundary without claiming managed-value lowering is complete.
- The aggregate Cranelift carrier remains intentionally gated: general
  `ArrayLoad` must preserve both words and register object/string payloads with
  precise GC metadata before it can leave Tier 0. A narrow single-pointer
  object-array load/store ABI now leaves Tier 0 with a managed write barrier;
  mixed and aggregate `Any` array operations remain on the interpreter path.
  The Cranelift method signature and shim now preserve an `Any` return as two
  `i64` values, and `call_packed_any` is separate from the scalar `u64` API;
  its parameter layout also accepts each `Any` argument as the same two-word
  `(tag, payload)` carrier and roots object-tagged payloads at the host
  boundary. Internal calls, aggregate lowering, and eligibility remain gated
  until the carrier is propagated end to end. A narrow direct
  `Any`-parameter-to-`Any`-return method now uses the same two-word shim and
  is covered by equivalence tests; aggregate values and conditional carrier
  propagation remain gated. A narrow direct `Any`-to-`Any` call chain is also
  covered, with carrier materialization in caller and callee stack slots.
- Interpreter safepoints already poll allocation sites and backedges with the
  current frame and caller chain as precise roots. JIT allocation calls now
  pass the pre-call SP, derive the x86-64 return PC, and publish the stopped
  native frame against the Cranelift registry during the allocation slow path.

The following runtime contracts are implemented and covered by tests:

- compact virtual-slot dispatch in both the interpreter and Cranelift JIT;
- O(1) field metadata lookup and per-type virtual tables;
- behavior-preserving module splits for the interpreter native bridge and
  managed-builtin resolution;
- behavior-preserving concern splits for core runtime contracts (dynamic values,
  symbols, function handles, dates, buffers, arrays, collections, and stable
  identifiers), semantic capture/mutation analysis, and interpreter GC roots;
- mark-sweep GC with interpreter roots, native root scopes, and precise JIT
  stack-map reference-slot metadata; object-producing JIT values are registered
  in those maps and the GC can consume a stopped native frame through `(PC, SP)`.
  JIT allocation, object-initialization and field-store barriers, managed calls,
  and imported builtins
  now retain object operands across their call boundaries, and field loads
  retain their receivers in stack maps; nested native-frame collection
  coverage is tested; a regression also proves a JIT reference store retains a
  young target through minor collection. Interpreter safepoints now select
  minor/full collection policy, and the GC boundary has RAII thread-local
  registration for nested stopped frames; the Cranelift host boundary can
  publish caller-supplied `(PC, SP)` coordinates, and ordinary mark-sweep and
  compaction can consume every registered frame at the single-thread
  allocation safepoint;
- the heap exposes a read-only compaction-plan snapshot and an opt-in moving
  compaction path. `RelocationMap` provides forwarding-address translation;
  metadata-declared object fields, interpreter roots, native roots, stopped JIT
  frames, strong side tables, generators, views, weak metadata, promises,
  proxies, and runtime carriers are rewritten before old chunks are released;
  DataView buffer fields are explicitly rewritten;
  iterator/typed-view side tables and weak metadata translation are covered,
  while weak liveness remains a separate phase. Active native root scopes and
  stopped frames can now rewrite their reference slots through the forwarding
  map; `Heap::rewrite_all_references` coordinates the heap-owned passes, and an
  explicit stop-the-world `Heap::compact` now copies live objects into fresh
  chunks and releases the old chunks. In the current single-thread runtime,
  the allocation helper is the safepoint: it captures `(PC, SP)` and consumes
  the registered JIT stack map. Multi-thread suspension remains deferred;
  normal full-GC integration now compacts and rewrites top-level interpreter
  stacks/locals at safe points; nested live borrowed interpreter frame chains
  remain on the conservative mark/sweep path until they can be rewritten as a
  unit. Owned suspended interpreter frames now expose a
  recursive rewrite path; object-tagged `Any` values are included in precise
  frame roots, and the opt-in compaction path exposes a forwarding callback so
  owned suspended frames can rewrite their values before old chunks release;
  `GeneratorSession` exposes this maintenance operation and its cross-resume
  object-state equivalence is tested.
  An opt-in
  `collect_and_compact` API now runs mark/sweep followed by that explicit
  movement path for interpreter roots and active `NativeRootScope` roots.
  `compact_with_native_frame` and `compact_with_native_frames` rewrite stopped
  JIT frames from their `(PC, SP)` stack maps, and
  `compact_with_registered_native_frames` consumes the active-frame registry
  without requiring the coordinator to rebuild the frame list. Ordinary
  mark/sweep, young collection, and `collect_and_compact` now also scan those
  registered frames and preserve their slots through relocation. The coordinator-facing explicit frame-list API, the thread-local
  convenience path, and the heap-backed JIT call boundary now share the same
  mark/relocate implementation; packed and zero-argument JIT entry boundaries
  can publish the coordinator's `(PC, SP)` pair, with packed calls also
  supporting a host that already installed the current heap scope.
- Promise/microtask scheduling and scalar external-library symbol calls;
- `throw`/`try`/`catch`/`finally` handler regions, verifier validation, nested
  Tier 0 unwinding, cross-frame propagation, and explicit JIT fallback for
  exception-bearing methods;
- `Any` and `DynamicObject`, including dynamic-property IL, source lowering,
  checked `castclass` lowering from `Any` to class instances, and GC tracing
  of object-tagged dynamic values;
- interpreted backedge observation plus Tier 1 local backedge poll counters as
  the first OSR integration boundary; hot `(method_rid, target_pc)` transfer
  requests are published and deduplicated by `JitHook`, and the hook now
  captures the Tier 0 PC, evaluation stack, and locals for each hot boundary.
  Snapshots validate method-local layout and IL boundaries before consumption,
  reject non-empty evaluation stacks, and reject protected/handler intervals
  using the .NET patchpoint restrictions; protected-region patchpoints are not
  published. References retained by pending snapshots participate in
  interpreter GC roots, while a Cranelift entrypoint accepting the captured
  frame state and native frame replacement remain pending.
- Cranelift relocatable object emission for the JIT-eligible subset, with the
  `.zm` module retained as the fallback for ineligible methods.
- Cranelift managed-heap entry boundaries now expose explicit
  `call_packed_with_heap` and `call_entry_with_heap` APIs. The eligible subset
  covers tested numeric array allocation/length, numeric object fields, and
  object-reference field loads/stores with interpreter/JIT equivalence.
  ArrayBuffer now has a dedicated managed type and its non-mutating
  `bufferLength` query, scalar reads (`bufferGetU8`, `bufferReadU32`,
  `bufferReadF64`), and scalar writes (`bufferSetU8`, `bufferWriteU32`,
  `bufferWriteF64`) cross the same object ABI. Builtin calls now support
  position-specific argument widths where a managed intrinsic mixes receiver,
  offset, and element types.
  Map, Set, and WeakMap
  receivers also have that ABI for read-only size queries; numeric `Map.has`
  and `Set.has` now use typed `f64` membership helpers; integer `i64` Map/Set
  membership, insertion, deletion, and numeric reads now use the same typed
  boundary; `Set.add(f64)` also
  crosses the scalar mutation boundary, while generic keys, values,
  `Map.set(f64, f64)` now returns an explicit insertion flag through the same
  scalar ABI, and `Map.delete(f64)` and `Set.delete(f64)` use it as well, while
  `Map.getF64Or` provides a typed numeric read with an explicit fallback.
  Other mutating methods remain on the interpreter path. Generic `Map.get`
  now has a narrow non-allocating out-pointer `Any` ABI for direct methods,
  including mixed scalar-return calls and interned-string payloads; broader
  managed aggregate values and generic mutation still require the remaining
  precise-GC and write-barrier work.
  `DataView` now has a distinct native receiver type, and its scalar
  `dataViewGetU32`, `dataViewGetU16`, and `dataViewGetF64` reads, plus scalar
  writes `dataViewSetU32`, `dataViewSetU16`, and `dataViewSetF64`, are covered
  by the same JIT object boundary.
  `Uint8Array`, `Uint16Array`, `Uint32Array`, and `Float64Array` now have
  distinct native receiver types and scalar `get`/`set` operations through
  that boundary; aggregate values and DataView construction remain separate
  managed-runtime gates.
- Primitive-backed `String` scalar queries (`length`, `contains`, `charCodeAt`,
  `startsWith`, `endsWith`, `indexOf`, `lastIndexOf`, `count`, and `compare`)
  now use direct Cranelift helpers with interpreter/JIT equivalence coverage.
  Managed string-producing operations (`concat`, `substring`, `trim`,
  `trimStart`, `trimEnd`, `toLowerCase`, `toUpperCase`, `repeat`, `reverse`,
  `padStart`, `padEnd`, and `replace`) also use heap-handle helpers with
  equivalence coverage. Aggregate/dynamic values remain on the interpreter
  path.
- Interned string handles now have typed JIT coverage in object fields:
  field reads, field stores, and constructors receiving string handles
  preserve interpreter/JIT equivalence. Literal-pool construction remains on
  the interpreter path until relocatable literal storage is available.
- The scalar string conversions `toInt`, `toFloat`, and `toBoolean` now have
  direct `CallBuiltin` Cranelift helpers with equivalence coverage. Their
  instance-method form is also explicitly bound to `zun:string` in semantic
  analysis, so primitive-backed String calls do not collide with homonymous
  Number methods.
- A minimal `zun_napi` C ABI host boundary that executes compiled `.zm` modules
  without exposing compiler or AST types, with typed integer, floating-point,
  boolean, and UTF-8 string result APIs plus size-query string copying.
- An optional `node-api` feature exporting `napi_register_module_v1` with
  `runI64`, `runF64`, `runBool`, and `runString` ArrayBuffer/Buffer-backed
  JavaScript functions. Invalid input and VM/type failures become JavaScript
  TypeErrors. Linking the shared library requires the host's N-API import
  library through `ZUN_NODE_LIB` on platforms that require explicit symbol
  imports.
- Typed scalar FFI calls are exercised against a real Windows system library;
  logical-library candidate resolution, aggregated load errors, and an
  absolute-path policy are implemented. `FfiSignature` now describes the
  supported blittable `i64`, `f64`, and zero-argument `bool` C shapes, and
  validates them before dispatching to fixed-arity thunks. Compiler/IL
  external `MemberRef` calls now carry their arity and scalar parameter/return
  tags and can be intercepted by a host `CallHook`; `zun_vm` provides an
  explicit `NativeCallHook` and `execute_with_native` boundary that validates
  registered scalar signatures against that metadata before invoking typed
  thunks while retaining JIT tiering. Source-level P/Invoke-style declarations
  and non-blittable marshalling remain separate gates.
- A poll-driven `AsyncStateMachine` runtime contract with persistent state,
  single-assignment completion, Promise integration, rejection propagation,
  typed `Pending`/`Resolved`/`Rejected` state inspection, and
  microtask-scheduled success/failure continuations, settlement-preserving
  `finally` cleanup, and a readiness/continuation `PromiseAwaiter` protocol.
  Source-level
  `async`/`await` source lowering through a managed Promise carrier. The Tier 0
  path now preserves hoisted locals across suspension, has a synchronous
  completion fast path, propagates rejection, resumes nested caller frames, and
  rejects suspension inside exception handlers. The interpreter remains the
  correct fallback; Cranelift compilation of methods containing `await` is an
  explicit eligibility boundary. Broader Cranelift support for async and
  generator state machines remains a separate optimization gate.
- Generator lowering preserves `yield` through HIR and verified IL, accepts
  `function*` declarations, and resumes owned Tier 0 frames. Managed generator
  objects are GC-traced, expose `next()`/`done` intrinsics, and work with
  source-level `for...of` for argumentful generators. The interpreter is the
  explicit JIT fallback; broader Cranelift suspension support remains pending.
- Positional `CoreTypeId`/`CoreTypes` bindings resolved once while loading
  `zun:core`, with O(1) access to the frozen core-type anchors.
- Explicit `extern` method declarations bound to imported native functions;
  sema validates receiver/parameter/return signatures and lowers the method
  body to the existing `call.builtin` instruction.
- Machine-facing `zun:string` primitives (`length`, `concat`, `contains`, and
  `charCodeAt`) and `zun:number` conversion primitives (`parseFloat` and
  `toFixed`), all covered by native dispatch tests.
- Contiguous `ArrayStorage` with bounds-checked access and growth operations,
  integrated with heap allocation and precise object-reference tracing. The
  source-level `Array` type is recognized by sema/IL typing; `new Array(n)`,
  indexing, indexed assignment, `length`, `push`, and `pop` lower to verified
  IL and execute in Tier 0. Typed helpers live in `std/array.zun`.
- Array instructions have verifier fixtures and explicit interpreter/JIT
  equivalence coverage. Read-only `ArrayLength` and scalar `f64`/`u64`
  `LoadArray`/`StoreArray` now lower through Cranelift helpers; numeric
  allocation and indexed mutation use the narrow non-reference value ABI, while
  dynamic/reference-valued loads/stores and
  aggregate `Any` loads/stores remain Tier 0 pending the two-word managed-value
  ABI. Sema-proven scalar `UnboxAny` extraction is the narrow exception:
  `u64`, `f64`, and `boolean` carrier paths have direct interpreter/JIT
  equivalence tests;
  checked string/reference unboxing remains Tier 0. Object allocation,
  reference fields, precise allocation safepoints, and
  write barriers now have Cranelift lowering and interpreter/JIT coverage.
  Checked `UnboxAny` remains an explicit Tier 0 boundary because a tag mismatch
  can throw and Cranelift exception bridging is not implemented; eligibility is
  governed by the no-throw/exception-bridge rule rather than per-array-type
  allowlists. Checked `CastClass` follows the same rule and remains Tier 0 until
  the Cranelift exception bridge exists.
- Runtime-backed ordered hash storage for strong `Map` and `Set`, including
  SameValueZero lookup, stable object-key hashing, insertion order, and GC
  tracing of managed keys and values. Source-level `Map` and `Set` classes
  delegate to these intrinsic bindings.
- Per-isolate `SymbolRegistry` with unique symbols and canonical
  `Symbol.for`-style keys; source-level `Symbol.iterator` calls resolve to
  managed Array, Map, and Set iterator handles.
- GC weak-reference slots with post-mark clearing; `WeakMap` ephemeron
  fixed-point marking is now implemented for managed object values and their
  dynamic/collection children; finalization
queues are now exposed as a post-sweep token queue with stable registration IDs
and cancellation before enqueue; source-level `WeakMap` bindings are available
while callback scheduling remains a separate gate.
- adaptive interpreter GC intervals that increase when a collection reclaims
  nothing, with weak-collection stress coverage at repeated safepoints;
  generational remembered-set coverage now includes raw fields, arrays,
  dynamic properties, Map/Set entries, Promise results, and WeakMap writes,
  with minor-collection retention tests; moving compaction now rewrites heap,
  native-frame, interpreter-frame, generator, promise, proxy, iterator, and
  typed-view references through the relocation map;
- Contiguous `ByteBuffer` storage with bounds-checked heap access, forming the
  intrinsic substrate for `ArrayBuffer` and typed-array views. Little-endian
  `u16`, `u32`, and `f64` view operations are covered; source-level view
  constructor bindings are available through the interpreter path. `ByteView` now enforces bounded
  shared windows without copying the underlying storage, and `TypedArrayView`
  enforces element alignment and indexed `u8`, `u16`, `u32`, and `f64` access
  over those windows. `DataView` now provides mixed-width, unaligned,
  little- or big-endian `u8`, `u16`, `u32`, and `f64` access over the same
  bounded storage; broader typed-array method classes remain a separate
  integration gate.
- Managed millisecond `DateValue` storage with invalid-date propagation and
  comparison semantics, plus UTC Gregorian date decomposition; calendar
  formatting/parsing are now covered for `YYYY-MM-DD`; source-level
  constructors remain follow-up work. UTC time-of-day decomposition and strict
  `YYYY-MM-DDTHH:MM:SS.mmmZ` formatting/parsing are also available through
  heap-level Date accessors, including timestamp mutation and invalidation.
  UTC weekday and ordinal-day accessors are also available, including
  runtime-clock construction through `alloc_date_now`.
  A `zun:date` native surface now exposes deterministic UTC year, month, day,
  hour, minute, second, millisecond, weekday, and ISO-date/datetime operations
  over millisecond timestamps, with a
  source-level `Date` class for the interpreter path. The source surface now
  includes timestamp factories, mutation/value aliases, comparisons,
  ECMAScript UTC aliases, ISO formatting, and ISO date/datetime parsing with
  invalid-input rejection. JIT lowering remains a separate gate for the native
  date intrinsics.

The typed Cranelift backend now lowers numeric `zun:date`, `zun:math`, and
`zun:number` predicates through stable C ABI/import entries, with eligibility
and interp/JIT equivalence coverage. String-returning Date calls, string
operations, and other managed-value builtins remain Tier 0 until their value
representations and ABI lowering are defined. The first string exception is
now explicit: interned string handles have a stable `JitKind::String` ABI and
the allocation-free `zun:string` `length` intrinsic is lowered through
Cranelift with equivalence coverage. String-producing operations and
primitive `String.length()` dispatch now use that same direct handle path.
String-producing operations and error-producing indexed operations remain Tier
0 until the JIT exception/landing-pad ABI is available.

The source-level core library now includes tested instance methods for String
classification/validation and quoting, Number predicates/interpolation, and
Array mutation, ordering, set operations, and numeric aggregation. These
methods deliberately remain ordinary Zun source and preserve the interpreter
fallback/equivalence boundary. Higher-order Array `map` and `filter` now
execute in Tier 0 through limited unary `Function` and `Predicate` callback
contracts and have interpreter coverage. `reduce` now uses the explicit
binary `Reducer` contract in Tier 0; captures and equivalent JIT lowering
remain separate gates.

Named function references and anonymous/arrow closures now lower through
`ldftn`/`mkclosure` and `call.indirect` and execute in Tier 0; Cranelift
closure-call lowering remains a separate optimization gate.

Named callback parameters remain exact and validate both parameter and return
signatures. The generic `Function` and `Predicate` types are now available as
deliberately limited unary callback contracts: `Function` returns `any`, while
`Predicate` returns `boolean`, and `Reducer` accepts two arguments and returns
`any`. Capturing callbacks are supported in Tier 0 with typed generic
signatures. Their indirect calls use typed null tokens to defer the target RID
to the handle; Cranelift closure-call lowering remains a separate optimization
gate. The verifier and loaded-module resolver cover the unary and binary
null-token forms.

The closure ABI is implemented as a coordinated gate: a closure environment is an
ordinary managed object whose fields are the captured locals in declaration
order; `FunctionHandle.environment` points to that object; and an indirect
closure call supplies the environment as an implicit first argument to the
generated method. Closure construction, verifier stack effects, environment
field load/store lowering, interpreter dispatch, and GC root tracing are
implemented; lambda syntax is enabled for Tier 0.
The first construction opcode, `mkclosure`, now consumes one managed object
and produces a `FunctionHandle`; `call.indirect` now prepends that environment
for generated closure methods whose body declares the hidden parameter.

The managed error hierarchy is now declared in `std/error.zun` with `Error`
and the standard ECMAScript error subtypes. Throwing, stack capture, and
constructor lowering remain runtime/compiler gates. The source hierarchy now
has positional `name` and `message` fields that work through inherited class
layouts, plus a source-level `toString()` implementation. The heap also stores
managed `ErrorValue` name/message pairs with mutation and sweep cleanup,
providing the runtime backing for those source-level error objects.

The release benchmark rerun on 2026-10-21 measured 435.190 ns/iteration for
the integer Tier 0 loop and 3.956 ns/iteration for the equivalent JIT loop
(110.45x on the 2026-10-21 rerun). The metadata-focused Tier 0 workloads
measured 279.251 ns/iteration for field load/store and 529.247 ns/iteration for virtual
dispatch. The result was identical between interpreter and JIT. These figures
are machine-specific and should be re-run after interpreter or OSR changes;
the benchmark source is
`crates/zun_codegen/examples/bench.rs`.

The narrow scalar array ABI is also implemented: Cranelift lowers `LoadArray`
and `StoreArray` for `f64`, `u64`, and `boolean` values through heap-scoped
helpers, with interpreter/JIT equivalence coverage. The same scalar boundary
covers `Array.push` and `Array.pop` for all three scalar kinds. `Any`-valued
and reference-valued array access remains Tier 0 until the two-word
managed-value ABI and its precise GC/write-barrier contract are complete.

The same boundary also covers managed-object `Array.push`/`Array.pop` through
the pointer ABI, including
  hot calls through `JitHook` with the active interpreter heap. The object
  stack-operation path has direct interpreter/JIT equivalence coverage.

The dynamic array ABI now has validated load and store helpers for the explicit
two-word `Any` carrier. Proven scalar `UnboxAny` reads use a temporary
two-word carrier slot in Cranelift before extracting the payload; checked
reference/string unboxing and general carrier arguments/returns remain Tier 0.
Object payloads are covered by a GC root test through the containing array;
aggregate carrier lowering remains intentionally gated until conditional
reference metadata can be represented safely.
Generic `Map.has`/`Set.has` consumers use the same carrier-shaped native
signatures but remain explicit Tier 0 boundaries until Cranelift verifies the
carrier extraction and call ABI; the interpreter equivalence tests preserve
the behavior while this lowering is deferred.

The GC now records a young/old generation on every live allocation and promotes
survivors at full-collection boundaries. Interpreter and Cranelift reference
stores, plus array, map, set, promise, proxy, weak-map ephemeron, and closure
environment side tables, publish old-to-young remembered-set barriers.
`Heap::collect_young` reclaims unreachable young objects, promotes survivors,
preserves old objects, and clears dead young weak references. Interpreter GC
safepoints now use seven minor collections followed by a full collection, and
finalizer callback environments are explicit roots during both collection modes.
This completes the first generational policy while leaving nursery allocation
and card-table scaling as later optimization work. The young-only correctness
path is no longer pending; incremental scheduling and larger-scale
nursery/card-table tuning remain. JIT allocation safepoints now
capture the x86-64 return PC from the pre-call SP, publish the stopped frame,
consume the corresponding Cranelift stack map during collection, and compact
the heap at a conservative periodic boundary while rewriting registered JIT
slots. Multi-threaded stop-the-world coordination remains deferred until the
VM has multiple mutator threads.

The runtime floor now also contains a normalized sign-magnitude `BigInt`
implementation with decimal parsing and exact add/subtract/multiply/compare
operations. It is intentionally not exposed through the language yet: the
managed carrier, GC tracing contract, literals, verifier, and interpreter/JIT
lowering must land as one later vertical slice.

The remaining completion gates are broader JIT lowering for managed heap values,
additional platform coverage for the automatic collection policy at the
now-published precise JIT-frame safepoint, true OSR frame replacement, and
 non-blittable FFI marshalling (deferred; its ownership ABI is specified in
 section 4.4), production Node N-API bindings, AOT/R2R support
 for the full runtime surface, BigInt, and the remaining Group 3 type-level
features. Runtime JIT string literals now use an explicit literal-handle ABI:
the JIT owns stable UTF-8 bytes and interns them through the active heap scope;
the interpreter/JIT equivalence path covers literal `String` methods. The
runtime JIT now also carries those literals through `newobj` and typed string
fields. The relocatable/AOT backend now emits `ConstStr` literals into
relocatable Cranelift data objects rather than embedding process-local pointers.
The AOT launcher still requires the managed runtime/helper bridge for
string-returning entrypoints and remains restricted to scalar returns.
Parameter/field string handles use the existing typed ABI. RegExp, FinalizationRegistry, and the Proxy/Reflect dynamic path
are implemented with Tier 0 fallback. Each gate requires a runtime test
and an interpreter/JIT equivalence test where JIT execution is applicable.

Behavior-preserving parser concern splits now cover helper extraction for
function expressions, control-flow statements, expression precedence, calls,
arguments, member/index access, primary expressions, and template processing;
the parser unit tests now live in `crates/zun_syntax/src/tests.rs`.

## 6. Execution backlog and rules (active goal — 2026-10-20)

This is the standing goal the implementation agent executes, part by part, until
the whole backlog is done. It is mirrored by `GOAL.md` at the repo root.

Rules (every slice):

- atomic commit per sub-step; `cargo test --workspace` green before moving on;
- the Tier 0 interpreter is the always-correct fallback; interpreter↔JIT
  equivalence is required for any new IL;
- opcodes only via the master table; every new opcode ships with
  `_Valid`/`_Invalid_<Error>` verifier fixtures;
- reference types/members by metadata token, never by offset;
- data structures with real algorithmic complexity (hashing, buffers) live in
  the Rust intrinsic floor; the TypeScript class is a thin wrapper over it,
  never TypeScript-over-`Array`;
- English everywhere; mark commits that touch GC, closure capture, or exception
  unwind with `GC:` / `closure-capture:` / `unwind:` in the first line;
- never leave the working tree dirty between commits.

Ordered backlog:

1. **Closures — complete in Tier 0** (arrow / anonymous functions): HIR synthetic function with an
   explicit environment parameter; sema free-variable analysis with
   capture-by-binding (a captured variable that is mutated becomes a boxed heap
   cell shared with the enclosing scope); ilgen lowering to `mkclosure` with GC
   tracing of the environment; then flip sema to accept the syntax and wire
   `Array` `map`/`filter`/`reduce`/`forEach` to inline arrows. Tests: map with an
   arrow, capture of an outer variable, mutation-after-capture visible in the
   closure, nested closures; interpreter↔JIT equivalence.
Status: **complete**. The Tier 0 path includes HIR synthetic
functions, binding captures, boxed mutable locals, nested environments,
`mkclosure` lowering, and GC tracing. Broader JIT closure-call lowering remains
an optimization gate.

2. **Iteration protocol — complete**: Array `for...of` is implemented; managed iterator
   handles now exist for Array, Map, and Set, preserving insertion order and
   Map entry-pair semantics. First-class source `Symbol.iterator` calls now
   dispatch to those managed handles through the interpreter fallback.
3. **Built-in class surface — complete**: `Map`, `Set`, and `WeakMap` constructors plus
   thin source classes and intrinsic receiver bindings are implemented. Source bindings now cover
   managed `DataView` and `Uint8Array`, `Uint16Array`, `Uint32Array`, and
   `Float64Array` views over GC-owned buffers.
4. **Weak-GC hardening — complete for the current runtime contract**: multi-entry ephemerons, address-index restoration
   after block reuse, key-aware managed lookup, and repeated-cycle stress
   coverage are implemented. The source-level `FinalizationRegistry` wrapper,
   registration IDs, unregister, and closure callback preservation are now
   available through Tier 0; callbacks are delivered at both normal and
   resumable-session completion boundaries, draining cascading registrations
   until the queue is empty.
5. **Group 2 syntax — complete for the planned current subset** (section 4.4.2): implemented through template
   literals, including spread/rest, destructuring, default parameters,
   optional property/method chaining, nullish coalescing, `switch`, loops,
   short-circuit operators, ternary expressions, and labelled loop control.
   Static methods are complete through parser, sema, metadata, verifier, and
   direct-call lowering. Static fields are complete through source syntax,
   metadata, Tier 0 storage, GC roots, and `ldsfld`/`stsfld`; getters/setters
   now lower from declarative `get`/`set` syntax to ordinary accessor methods;
   private `#fields` use type-checked mangled field layout and ordinary field
   IL lowering.
6. **Remaining architecture**: broader managed Cranelift values, true OSR frame
   replacement, non-blittable FFI marshalling (deferred; ownership modes are
   specified in section 4.4), production N-API, AOT/R2R, and the
   remaining performance backlog (the first OrderedMap and generational-GC
   policies are complete; only their measured scaling follow-ups remain).
   Async/await source lowering is complete in
   the Tier 0 state-machine path; generator source lowering, managed iterator
   objects, `next()`, and `for...of` integration are implemented, including
   argument arrays. Broader JIT support remains pending. Module-hygiene splits are
   maintained separately by the reviewer and are not part of this execution
   backlog.
7. Keep this section and the section 5 ledger up to date as items land.

The pre-OSR eager-JIT step is complete: `JitHook` compiles every eligible
method containing a backedge at its normal entry point before execution. This
recovers Tier 1 execution for whole loop methods without transferring a
suspended interpreter frame. True mid-frame OSR remains deferred until the
planned partial-method/live-in ABI is implemented.

Stop condition: stop only on a real environment blocker, a design decision not
covered by the plan, or completion of the whole backlog.

## 7. Performance backlog (measured 2026-10-20)

### 7.0 OXC front-end prototype (measured 2026-10-21)

An isolated prototype using `oxc_parser` 0.155.0 successfully parsed a
TypeScript function with the existing Zun-style type annotation. A second
prototype adapted top-level OXC function declarations into a small HIR-shaped
record (name, parameter count, async, generator), covering the first adapter
boundary. Both prototypes pulled 77 crates; the clean adapter build took
56.750 s, the release executable was 893,952 bytes, and the measured
incremental build/run was 4.954 s on the development machine. No dependency
was added to the workspace: the adapter to the real Zun HIR is a separate
front-end track and must be introduced behind an explicit integration boundary
after broader AST-to-HIR coverage and cold-start cost are measured.

Optimizations found by benchmarking. These are **not** correctness or complexity
defects — they are constant-factor / engine costs to improve after the section 6
backlog. Both measured on this date.

### 7.1 `OrderedMap` insert constant (Map/Set floor) — addressed

Direct benchmark of `zun_core::OrderedMap` (the floor under `Map`/`Set`), insert
then probe, at N=200k and N=1M (5x apart):

| N | insert | probe |
|---|---|---|
| 200,000 | 417 ns/op | 33 ns/op |
| 1,000,000 | 428 ns/op (x1.03) | 40 ns/op (x1.22) |

Complexity is confirmed **O(1)** (per-op is flat across a 5x N; it is not an O(n)
scan). The runtime now uses open addressing with one contiguous bucket array,
while the insertion-order entry vector remains independent of bucket layout.
Collision probing, tombstone reuse, compaction, and SameValueZero callbacks are
covered by core tests. A Robin Hood/Swiss-table probe policy remains an optional
constant-factor follow-up, not a correctness or complexity gate.
Bucket capacities are powers of two and probe advancement now uses a bit mask;
this preserves the current layout and semantics without asserting a benchmarked
speedup.
The reproducible workspace workload is `cargo run --release --example
ordered_map_bench -p zun_core`; the 2026-10-21 rerun measured `N=200000` at
31.724 ns/op for insert and 4.343 ns/op for probe, and `N=1000000` at
31.942 ns/op for insert and 2.966 ns/op for probe. These numbers are a new
machine-specific baseline, not a before/after comparison.

### 7.2 Tier 0 interpreter GC is O(n^2) over a run

A Map/Set benchmark run *through compiled Zun code* (interpreter) did not finish
in 420s for N=500k, while the data structure itself handles 1M in well under a
second. The cost is the **interpreter garbage collector**, not the collection:
mark-sweep fires every `GC_INTERVAL` (256) allocations and scans the whole live
set each time, so a program that keeps O(n) objects live pays roughly
O(n^2 / 256) in GC over the run. For large live sets this dominates everything.
The adaptive GC interval is implemented: it scales the next safepoint with the
live-set and increases the interval when a collection reclaims nothing. This
now also increases the nursery budget when the observed young survival rate is
high. This removes the fixed-256 policy, but does not eliminate the underlying whole-heap
  scan. The current collector already has a young/old policy, remembered-set barriers,
  and an opt-in moving compaction path with relocation of interpreter values and
  registered native/JIT roots. A fully incremental/generational collector remains the
  long-term fix for avoiding repeated whole-heap scans. This remains the real performance
ceiling for large programs today, independent of any individual data structure.

### 7.3 Benchmarked opportunity sizing (2026-10-20)

Five further ideas, each measured in an isolated worktree before being recorded
(`#2`/`#4` on the real pipeline; `#1`/`#3`/`#5` on optimizer-resistant models).
Ranked by payoff:

| Idea | measurement | verdict |
|---|---|---|
| **Generational GC (nursery)** | full mark-sweep is O(n²) over a run (4× N → **16×** time); a nursery that scans only the young is **~22× faster** at N=40k and the gap grows with N | **highest** — the real ceiling (§7.2); a gen0 with bump alloc + cheap minor GC (doc 09/01, 09/03) is the fix, not just the adaptive interval |
| **OSR / eager-JIT-for-loops** | the 2026-10-21 rerun measured **290.289 ns/iter** in Tier 0 vs **2.628 ns/iter** in Tier 1, a **110.45×** gap | **eager-JIT complete** — eligible loop methods now compile at normal entry; true OSR (doc 06) remains deferred until frame-transfer ABI and precise live-in maps exist |
| **Interpreter superinstructions** | fusing a hot opcode sequence on the same tagged-stack VM: **~4–5×** fewer-dispatch speedup | **medium** — helps startup, cold code, everything before it is hot; stable Rust (no computed-goto); pairs with a leaner stack value |
| **Cache the compiled std/prelude (R2R-lite)** | compiling a trivial program costs **~39 µs**, almost all of it the std/prelude, **paid at every process start** | **medium/low effort** — persist the compiled std `.zm` (the format already reserves an R2R extension header, doc 02/05) for fast cold start |
| **Escape analysis to elide `any` box/unbox** | scalar `Any` box/unbox overhead is **~0.03 ns** (negligible — `Any` is a value type) | **low for scalars** — not worth a pass on its own; the real dynamic cost is heap object allocation, which is covered by generational GC (#1), not escape analysis |

Ledger clarification: the first generational policy is complete (young/old
classification, remembered-set barriers, and young-only collection). The
remaining performance work is nursery bump allocation and larger card-table
scaling; the table's measured ceiling must not be read as an unimplemented
minor collector.

Takeaway: the two big wins (generational GC and getting hot loops out of Tier 0)
are already in the section 6 cluster — this confirms their priority with numbers.
Superinstructions and the std cache are good medium wins. Scalar box/unbox
elision is **not** worth pursuing on its own.

## 5. Riscos e pontos de atenção

- **Semântica JS vs modelo estático**: o escopo do que `any`/JS dinâmico suporta precisa ser decidido cedo (não dá para ser 100% ECMAScript sem reconstruir um V8). Recomendação: definir Zun como "TS sound-por-padrão, com `Any` como opt-in custoso" e documentar divergências.
- **Cranelift não tem GC statepoints maduros**: usar safepoints por polling + stack maps via `cranelift` reference types ou mapas próprios — decidir na Fase 3, mantendo o interpretador como fallback sempre correto.
- **Verificar cedo**: o verificador de ZIL deve nascer junto da IL (fixtures `_Valid`/`_Invalid_*`, doc 08), não depois.

---
*Plano gerado a partir da análise de `.docs-dotnet/`; cada seção cita a pasta correspondente para aprofundamento.*
Implementation note (2026-10-20): the managed-object JIT ABI now also supports
non-allocating length queries for `ArrayBuffer`, `DataView`, `Uint8Array`,
`Uint16Array`, `Uint32Array`, and `Float64Array`, with interpreter/JIT
equivalence. View construction remains a Tier 0 fallback until precise
native-frame safepoints are connected.
The same boundary now distinguishes element `length` from byte `byteLength`
for `DataView` and all four typed-array view kinds.

The prelude now exposes a deterministic content fingerprint for the future
R2R-lite cache. Cache reuse remains deferred until the prelude is emitted as a
separate module with stable metadata/RID boundaries; the current concatenated
source path must not be cached as if it were independent.
