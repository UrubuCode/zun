# Zun — Plano de Arquitetura

Máquina de linguagem JS/TS em Rust, inspirada na arquitetura do .NET (CLR/CoreCLR), usando **Cranelift** como backend de código nativo. Não é "mais um runtime JS estilo V8": é uma plataforma com **IL própria, metadata, compile e runtime separados**, onde JS/TS é o front-end — como C# é para o .NET.

Base de referência: `.docs-dotnet/` (análise do dotnet/runtime, 10 áreas). As decisões abaixo seguem as recomendações desses documentos.

---

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

## 4.1 Modos de distribuição (decisão: suportar os três)

O usuário final nunca instala um runtime. Os três modos são empacotamentos do mesmo pipeline — o codegen atrás do trait estilo `ICorJitInfo` serve JIT (`cranelift-jit`) e AOT (`cranelift-object`) sem saber a diferença:

1. **Self-contained JIT (default do `zun build`)** — executável único: apphost nativo + VM linkada estaticamente + `.zm` embutido como seção de dados. Duplo clique e roda; JIT em runtime. Equivale ao single-file self-contained do .NET.
2. **Self-contained + R2R** — modo 1 com código nativo pré-compilado embutido (via `ObjectModule`) e ZIL como fallback para o JIT. Startup quase instantâneo, mantém dinamicidade.
3. **AOT puro (`zun build --aot`)** — Cranelift roda inteiro no build, linker do sistema gera o binário; runtime mínima embutida (GC, scheduler, exceções, estilo Go), mundo fechado com trimming e reflection reduzida (docs 08 e 02-compilation/07).

Restrições de plataforma que tornam o modo 3 obrigatório: **iOS e consoles proíbem JIT** (sem páginas executáveis); Android aceita JIT mas AOT dá melhor startup. Desktop (Win/Linux/macOS) pode usar qualquer modo. Ordem de entrega: 1 → 2 → 3, conforme doc 02-compilation.

## 5. Riscos e pontos de atenção

- **Semântica JS vs modelo estático**: o escopo do que `any`/JS dinâmico suporta precisa ser decidido cedo (não dá para ser 100% ECMAScript sem reconstruir um V8). Recomendação: definir Zun como "TS sound-por-padrão, com `Any` como opt-in custoso" e documentar divergências.
- **Cranelift não tem GC statepoints maduros**: usar safepoints por polling + stack maps via `cranelift` reference types ou mapas próprios — decidir na Fase 3, mantendo o interpretador como fallback sempre correto.
- **Verificar cedo**: o verificador de ZIL deve nascer junto da IL (fixtures `_Valid`/`_Invalid_*`, doc 08), não depois.

---
*Plano gerado a partir da análise de `.docs-dotnet/`; cada seção cita a pasta correspondente para aprofundamento.*
