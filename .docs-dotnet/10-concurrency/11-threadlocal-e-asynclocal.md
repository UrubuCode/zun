# ThreadLocal e AsyncLocal (estado por fluxo)

O .NET oferece três escopos de estado ambiente: **por thread** (`[ThreadStatic]`, `ThreadLocal<T>`),
**por fluxo lógico assíncrono** (`AsyncLocal<T>`, via `ExecutionContext`) e **global**. Este documento
descreve os dois primeiros e o mecanismo de fluxo de contexto que sustenta `AsyncLocal`.

## Mecanismo no .NET

### `[ThreadStatic]`

Campo estático com atributo `[ThreadStatic]`: o runtime aloca um slot por thread. É o bloco de construção usado
em várias partes do CoreLib, por exemplo `PoolingAsyncValueTaskMethodBuilder.t_tlsCache` (doc 08) e
`ThreadPoolWorkQueueThreadLocals.threadLocals` (doc 12). Não há inicialização por thread: o valor inicial é
`default`, e o código precisa checar `null` a cada acesso.

### `ThreadLocal<T>` (`Threading/ThreadLocal.cs`)

Cabeçalho do arquivo: implementação "simples e leve" de inicialização preguiçosa por thread, alternativa a
`[ThreadStatic]` que evita checar inicialização a cada acesso. Estrutura:

- **Tabela por thread, compartilhada entre instâncias**: `[ThreadStatic] static LinkedSlotVolatile[]? ts_slotArray` —
  um array com os valores de **todas** as `ThreadLocal<T>` do mesmo `T` naquela thread. A posição relevante
  para cada instância é o **ID** guardado em `_idComplement` (guarda `~ID`, para distinguir ID 0 de um objeto
  incompletamente construído).
- **`IdManager`** estático atribui e reutiliza IDs (lista de IDs livres, dicionário de IDs usados; protegido por
  `Lock`). O tamanho da tabela cresce (`GetNewTableSize(id + 1)`).
- Cada valor vive em um `LinkedSlot` (com `_slotArray` e `_value`); os slots de uma mesma `ThreadLocal<T>` formam uma
  **lista ligada** (`_linkedSlot`), usada por `Values` quando `trackAllValues: true` (enumeração de todos os valores
  de todas as threads) e por `Dispose`.
- **Caminho rápido do `Value`**: lê `ts_slotArray`, verifica `id < length`, lê `slotArray[id].Value` (`LinkedSlotVolatile`,
  leitura volátil — comentário: garante que a leitura de `m_initialized` não passe à frente) e retorna `_value` sem lock.
  O `GetValueSlow` chama a factory (`Func<T>`) e `SetValueSlow`/`CreateLinkedSlot` instalam o slot (sob lock).
- **Limpeza**: `FinalizationHelper` por thread (`[ThreadStatic] ts_finalizationHelper`) libera os slots da thread
  quando ela termina (se `trackAllValues`, remove da lista ligada); a `ThreadLocal<T>` tem finalizador
  (`~ThreadLocal`) que devolve o ID ao gerenciador. `Dispose` **não é thread-safe** (única exceção declarada).
- `IsValueCreated`, `Values`, `Dispose`, `ToString`, construtores com `valueFactory` e `trackAllValues`.
- Exemplo de uso real: `ConcurrentBag<T>` guarda sua fila local em `ThreadLocal<WorkStealingQueue> _locals`.

### Estado por fluxo: `ExecutionContext` e `AsyncLocal<T>`

Problema: após um `await`, o código pode retomar em **outra thread**; `[ThreadStatic]`/`ThreadLocal` seriam perdidos
(ou, pior, de outro fluxo). `AsyncLocal<T>` acompanha o **fluxo lógico** de execução.

**`AsyncLocal<T>`** (`Threading/AsyncLocal.cs`): `Value` get/set delegam para
`ExecutionContext.GetLocalValue(this)` / `SetLocalValue(this, value, needChangeNotifications)`. Construtor opcional com
callback `valueChangedHandler(AsyncLocalValueChangedArgs<T>)` chamado quando o valor muda, inclusive quando o
contexto é trocado (`contextChanged`).

**`ExecutionContext`** (`Threading/ExecutionContext.cs`), `sealed class`, **imutável**:

- Campos: `IAsyncLocalValueMap? m_localValues` (mapa `IAsyncLocal → object`), `IAsyncLocal[]? m_localChangeNotifications`,
  `m_isDefault`, `m_isFlowSuppressed`. Singletons `Default` e `DefaultFlowSuppressed`.
- O contexto vigente fica na própria thread: `Thread.CurrentThread._executionContext` (e `_synchronizationContext`).
- **Escrita copy-on-write**: `AsyncLocal.Value = x` cria um **novo** `IAsyncLocalValueMap` via `Set(...)`
  (retorna novo mapa) e um novo `ExecutionContext`. Quem capturou o contexto antes não vê a mudança — daí o
  isolamento entre fluxos e a propriedade "mudanças fluem para baixo (chamadas e continuações) mas não para cima".
- **Mapas especializados por tamanho** (`AsyncLocalValueMap.*`): `Empty`, `OneElement`, `TwoElement`,
  `ThreeElement` (campos fixos, comparação por referência), `MultiElement` (array até `MaxMultiElements = 16`),
  e `ManyElement` (dicionário, acima disso). Os casos de 1-3 elementos são os comuns, evitando alocar array/dicionário.
  Atribuir `null` pode remover a entrada quando não há notificações (`treatNullValueAsNonexistent`).
- **`Capture()`** devolve o contexto atual (ou `Default`; `null` se o fluxo está suprimido). `CaptureForSuspension`
  é usado ao suspender métodos async.
- **`Run`/`RunInternal(ctx, callback, state)`**: instala `ctx` na thread, invoca o callback, **restaura** o contexto
  anterior — usando `catch` (não `finally`) para restaurar *antes* de filtros de exceção dos chamadores rodarem;
  comentário: "extremamente quente, usado por todo await, execução do ThreadPool, etc.". Restaurar também o
  `SynchronizationContext` anterior. Dispara `OnValueChanged` dos `AsyncLocal` com notificação quando o contexto muda.
- **Supressão**: `ExecutionContext.SuppressFlow()` retorna `AsyncFlowControl` (descartável) e instala um
  `DefaultFlowSuppressed`/clone com flag; `RestoreFlow()` reverte; `IsFlowSuppressed()`. Usado para
  criar threads/timers/tarefas de infraestrutura sem herdar o contexto do chamador (ex.: `CancelAfter` com
  `TimeProvider`, threads do pool com `UnsafeStart`).

**Quando o contexto flui:**

- `await`: a caixa da máquina de estados captura (`CaptureForSuspension`) e, na retomada, roda via
  `ExecutionContext.RunInternal` (doc 03).
- `ThreadPool.QueueUserWorkItem` captura; `UnsafeQueueUserWorkItem` não (`QueueUserWorkItemCallbackDefaultContext`
  existe para o caso "sem contexto").
- `Task.Run`/`StartNew` captura em `ContingentProperties.m_capturedContext`; `Timer` com `flowExecutionContext`.
- `ThreadPool` workers começam e voltam ao contexto limpo entre itens (`Dispatch` na doc 12), o que pode
  chamar callbacks de mudança de `AsyncLocal`.
- `ValueTaskSourceOnCompletedFlags.FlowExecutionContext` controla isso para fontes customizadas (doc 08).
- Variantes `Unsafe*` (`UnsafeOnCompleted`, `UnsafeRegister`) **não** capturam.

### Comparação rápida

| Mecanismo | Escopo | Segue o `await`? | Custo de leitura | Observação |
|---|---|---|---|---|
| `[ThreadStatic]` | thread | não | muito baixo | sem inicialização por thread |
| `ThreadLocal<T>` | thread | não | baixo (tabela por thread + id) | factory lazy, `Values`, finalizador |
| `AsyncLocal<T>` | fluxo lógico | sim (via `ExecutionContext`) | busca em mapa pequeno | copy-on-write; notificações opcionais |
| Parâmetro explícito | chamada | sim | zero | verboso, mas sem mágica |

Nota: `SynchronizationContext.Current` e `ExecutionContext` são campos **da thread**
(`Thread._synchronizationContext`, `Thread._executionContext`) que o runtime troca a cada retomada; por isso
`AsyncLocal` funciona "por fluxo" mesmo estando armazenado na thread atual — o runtime instala o contexto
capturado ao retomar e o remove ao terminar.

## Por quê

- **Contexto implícito para observabilidade e identidade**: tracing/correlação, culturas, usuário atual,
  transações ambient — informação que o programador não quer passar manualmente em cada chamada.
- **Imutabilidade + copy-on-write** faz a captura ser uma cópia de **uma referência** (barata) e a leitura
  lock-free; essencial porque *todo* `await` captura e restaura.
- **Mapas pequenos especializados** otimizam o caso real (poucos `AsyncLocal` ativos).
- **Suprimir fluxo** evita vazar contexto (e referências) para trabalho de longa duração.
- **`ThreadLocal<T>` com slots compartilhados por tipo** poupa memória (um array por thread/tipo, não uma
  tabela por instância) e dá acesso rápido sem checagens por acesso.

## Trade-offs

- **Contexto implícito é "ação à distância"**: depuração mais difícil, dependências ocultas, risco de
  vazamento de dados entre requisições se mal usado.
- **Custo em toda transição**: capturar/restaurar `ExecutionContext` está no caminho de cada await e item do
  pool; o runtime investe em micro-otimizações (`RunInternal` com variáveis em registradores, evitar write
  barrier quando nada mudou).
- **Escrita copia o mapa** (O(n) para n entradas) — barato em n pequeno, mais caro em n grande.
- **Semântica sutil de "para baixo, não para cima"**: um método async que altera um `AsyncLocal` **não** afeta o
  chamador após `await` retornar (o chamador tem seu contexto antigo) — surpreende quem espera comportamento de
  variável global.
- **Notificações de mudança** (`valueChangedHandler`) forçam registro em `m_localChangeNotifications` e custo extra
  na troca de contexto.
- **`ThreadLocal<T>`**: mais lento que `[ThreadStatic]` simples; usa finalizador e ID manager global com lock;
  `trackAllValues` mantém valores vivos (e threads mortas podem reter memória até cleanup).
- **ThreadLocal/ThreadStatic não funcionam em código async** em geral — retomada em outra thread.
- Em pool de threads, valores `ThreadStatic` **vazam** entre itens de trabalho se não limpos (o pool limpa só
  `ExecutionContext`, não `[ThreadStatic]` do usuário).

## Como aplicar na sua linguagem

1. **Decida se quer estado ambiente por fluxo.** Se sim, modele-o como **mapa persistente/imutável** ligado ao
   "fluxo lógico" (tarefa, corrotina), com captura O(1).
2. **Capture na criação/suspensão e restaure na retomada**; a unidade de captura deve ser uma única referência.
3. **Mapas especializados** para 0-3 entradas e fallback para estrutura maior (o .NET usa limiar 3, depois array até
   16, depois dicionário).
4. **Escreveu → novo contexto**, nunca mutar contexto compartilhado. Defina claramente a regra de visibilidade
   (para baixo, não para cima) e documente.
5. **Primitiva de supressão** (`suppress_flow`) e variantes "unsafe" para infraestrutura.
6. **Limpe o contexto entre itens de trabalho** num pool (iniciar cada item com contexto limpo).
7. **Alternativa**: passe o contexto explicitamente (parâmetro implícito/ambiente tipado, como "context object"),
   evitando mágica; ou ofereça os dois. Avalie o custo de legibilidade versus ergonomia.
8. **Threads**: ofereça `thread_local` para dados realmente por thread (caches, alocadores); documente que não
   sobrevive a suspensão/retomada. Considere `thread_local` com inicializador lazy e destruição ao fim da thread
   (como `FinalizationHelper`).
9. **IDs de slot reutilizáveis** com gerente central, e leitura sem lock via tabela por thread.
10. **Teste de vazamento**: contexto capturado mantém objetos vivos; ofereça forma de limpar.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/AsyncLocal.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/ExecutionContext.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/ThreadLocal.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/SynchronizationContext.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/ThreadPoolWorkQueue.cs` (`QueueUserWorkItemCallback*`, `ThreadPoolWorkQueueThreadLocals`)
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/AsyncTaskMethodBuilderT.cs` (captura na suspensão)
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Sources/ManualResetValueTaskSourceCore.cs`
- `src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/ConcurrentBag.cs`
- `docs/design/coreclr/botr/threading.md` (variáveis thread-static)
