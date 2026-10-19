# docs-generated — Análise do .NET runtime para design de linguagem

Esta pasta é uma análise sistemática do **dotnet/runtime** (compile-time e runtime),
extraída para servir de referência ao projetar uma nova linguagem. Cada documento segue
o formato:

1. **Mecanismo no .NET** — como o runtime/compilador realmente faz (com caminhos de arquivo do repo).
2. **Por quê** — o racional de design por trás da escolha.
3. **Trade-offs** — custos e limitações da abordagem.
4. **Como aplicar na sua linguagem** — recomendação concreta de adaptação.
5. **Referências** — arquivos do repo e specs.

## Áreas

| # | Área | Pasta | Foco |
|---|------|-------|------|
| 01 | Resolução de módulos | [`01-module-resolution/`](01-module-resolution/) | AssemblyLoadContext, binding, deps.json, native libs |
| 02 | Compilação | [`02-compilation/`](02-compilation/) | IL, metadata, PE, R2R, Native AOT |
| 03 | Sistema de tipos | [`03-type-system/`](03-type-system/) | generics reificados, value/ref types, variância |
| 04 | Orientação a objetos | [`04-object-orientation/`](04-object-orientation/) | dispatch virtual, vtables, interfaces |
| 05 | Criação de objetos | [`05-object-creation/`](05-object-creation/) | newobj, layout, ordem de init, alocação |
| 06 | Blocos e controle de fluxo | [`06-blocks-control-flow/`](06-blocks-control-flow/) | EH, async/iterator state machines, escopo |
| 07 | Ações e operações | [`07-actions-operations/`](07-actions-operations/) | delegates, function pointers, call/callvirt |
| 08 | Análise estática | [`08-static-analysis/`](08-static-analysis/) | IL verification, nullable, trimming |
| 09 | Memória e runtime | [`09-memory-runtime/`](09-memory-runtime/) | GC, JIT, tiered compilation, spans |
| 10 | Concorrência | [`10-concurrency/`](10-concurrency/) | threads, Task/async, memory model |

> Gerado por análise com agents a partir do código-fonte e de `docs/design/`.
