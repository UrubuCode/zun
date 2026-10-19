# Channels e producer/consumer

`System.Threading.Channels` é a biblioteca de filas assíncronas produtor/consumidor do .NET: um canal
com leitores e escritores desacoplados, backpressure opcional, fechamento (`Complete`) e integração com
`await`, `CancellationToken` e `IAsyncEnumerable`.

## Mecanismo no .NET

Código em `src/libraries/System.Threading.Channels/src/System/Threading/Channels/`.

### API

- `Channel<T>` expõe `Reader` (`ChannelReader<T>`) e `Writer` (`ChannelWriter<T>`) separados — o produtor só
  recebe o `Writer`, o consumidor só o `Reader` (privilégio mínimo por tipo).
- `ChannelWriter<T>`: `TryWrite(item)` (síncrono, não bloqueia; falso se cheio ou fechado),
  `WaitToWriteAsync(ct)` (`ValueTask<bool>`), `WriteAsync(item, ct)` (`ValueTask`), `TryComplete(Exception?)` /
  `Complete(...)` (fecha a escrita; opcionalmente com erro).
- `ChannelReader<T>`: `TryRead`, `WaitToReadAsync(ct)` (`ValueTask<bool>`, false = canal completo e vazio),
  `ReadAsync(ct)` (`ValueTask<T>`), `ReadAllAsync(ct)` (`IAsyncEnumerable<T>`), `Completion` (`Task`),
  `CanCount`/`Count`, `CanPeek`/`TryPeek`.
- Todos os retornos assíncronos são `ValueTask` (doc 08): se o item está disponível, nenhuma alocação.
- Fábrica `Channel.cs`: `CreateUnbounded<T>()`, `CreateUnbounded<T>(UnboundedChannelOptions)`,
  `CreateBounded<T>(capacity)`, `CreateBounded<T>(BoundedChannelOptions[, Action<T> itemDropped])`.

### Opções (`ChannelOptions.cs`)

`SingleWriter`, `SingleReader`, `AllowSynchronousContinuations` (padrão falso) e, para limitados,
`Capacity` e `FullMode` (`BoundedChannelFullMode`): `Wait`, `DropNewest`, `DropOldest`, `DropWrite`.

### Seleção da implementação

```
CreateUnbounded(options):
    options.SingleReader ? SingleConsumerUnboundedChannel<T> : UnboundedChannel<T>
CreateBounded(options, itemDropped):
    options.Capacity > 0 ? BoundedChannel<T> : RendezvousChannel<T>
```

Existe também `UnboundedPrioritizedChannel<T>` (`UnboundedPriorityChannel.cs`), cópia adaptada do
`UnboundedChannel` usando `PriorityQueue<bool, T>` onde "a prioridade dobra como o elemento" para não armazenar
struct grande duas vezes; comentário avisa que o `PriorityQueue` não é thread-safe, então os caminhos
rápidos sem lock foram removidos.

### `UnboundedChannel<T>`

- Dados em `ConcurrentQueue<T>` (doc 07) — `TryWrite` e `TryRead` fazem a fila lock-free; o lock (`SyncObj`) só
  é usado para coordenar **leitores bloqueados/esperando** e conclusão.
- Listas ligadas de operações pendentes: `_blockedReadersHead` (`ReadAsync` pendentes) e
  `_waitingReadersHead` (`WaitToReadAsync` pendentes).
- `TryWrite`: se há leitor bloqueado, entrega o item diretamente a ele (`DangerousSetResult`) em vez de passar
  pela fila; senão `_items.Enqueue`, e acorda os "waiters" do `WaitToReadAsync`. Completar tasks acontece
  **fora do lock** ("now that we're outside of the lock").

### `SingleConsumerUnboundedChannel<T>`

Com `SingleReader = true` usa `SingleProducerSingleConsumerQueue<T>` e mantém só **um** `_blockedReader`
(não lista). Menos coordenação, mais rápido, mas o contrato exige um único leitor.

### `BoundedChannel<T>`

- Dados em `Deque<T>` protegido pelo lock `SyncObj` (`lock (parent.SyncObj)` em `TryRead`, `ReadAsync`, `TryWrite`…).
- Listas de pendentes: `_blockedReadersHead`, `_blockedWritersHead` (escritas esperando espaço, cada uma carrega
  o `Item`), `_waitingReadersHead`, `_waitingWritersHead`.
- **Leitura**: `DequeueItemAndPostProcess()`: tira da frente; se há escritores bloqueados, **promove o primeiro
  escritor não cancelado** (`w.TrySetResult` → `_items.EnqueueTail(w.Item)`), ou acorda `WaitToWriteAsync`;
  se `_doneWriting` e ficou vazio, completa `Completion`.
- **Escrita (`TryWrite`)** quando cheio depende de `FullMode`:
  - `Wait`: retorna falso (e `WriteAsync` bloqueia numa operação pendente → backpressure);
  - `DropWrite`: descarta o item novo e invoca `itemDropped`;
  - `DropNewest` / `DropOldest`: remove o mais novo/antigo para dar lugar e chama `itemDropped`.
- `TryComplete`: sob o lock marca `_doneWriting` (erro ou sentinela), captura e zera as listas de pendentes, e
  **falha** os escritores bloqueados com exceção `ChannelClosedException`; completa a task `Completion` quando vazio.
- `AssertInvariants` (só debug) documenta: "se há dados, não há leitores bloqueados"; "se há leitores, não há dados".

### `RendezvousChannel<T>`

Capacidade zero: cada escrita só completa quando há um leitor (encontro síncrono).

### `AsyncOperation` / `AsyncOperation<T>` (`AsyncOperation.cs`)

Cada espera pendente é um objeto que **implementa `IValueTaskSource<T>`** — o canal retorna `ValueTask`
baseado nele. Campos: `_continuation`, `_continuationState`, `_currentId` (versão/token), `_error`, `_result`,
`_pooled`, `_isCancelable`, `_cancellationRegistration`, `_completionReserved`, sentinelas
`s_availableSentinel` e `s_completedSentinel`.

- **Reutilização**: leitores e escritores criam **um "singleton"** de operação (`_readerSingleton`,
  `_waiterSingleton`, `_writerSingleton`) com `pooled: true`; quando a chamada **não é cancelável** e o singleton
  está disponível (`TryOwnAndReset`), ele é reutilizado em vez de alocar. Operações canceláveis nunca são pooled
  (`Debug.Assert(!_pooled, "Cancelable operations can't be pooled")`) e registram o token com
  `CancellationToken.UnsafeRegister`.
- **Completar uma vez**: `TryReserveCompletionIfCancelable` (`Interlocked.Exchange(ref _completionReserved, 1) == 0`)
  decide a corrida resultado × cancelamento. Cancelamento: `TrySetCanceled` armazena `OperationCanceledException`.
- **Continuação**: `Volatile.Read(ref _continuation)` e `Interlocked.CompareExchange(ref _continuation,
  s_completedSentinel, null)` — mesma técnica de `ManualResetValueTaskSourceCore`. Comentário: atribuir
  `_continuation` acontece depois de atribuir suas dependências (`_capturedContext`, `_continuationState`),
  e é preciso ler `_continuation` antes de acessá-las.
- **Sincronia da continuação**: com `runContinuationsAsynchronously` verdadeiro (padrão do canal,
  `!AllowSynchronousContinuations`), a continuação do consumidor é agendada no scheduler/pool em vez de rodar na
  thread do produtor dentro de `TryWrite`; caso contrário roda inline. O código captura `ExecutionContext` e
  `SynchronizationContext`/`TaskScheduler` conforme os flags de `OnCompleted`.
- Há variantes `.netcoreapp.cs`/`.netstandard.cs` para APIs de cada alvo.

### Padrão de consumo típico

```
await foreach (var item in channel.Reader.ReadAllAsync(ct)) { ... }   // consumidor
await channel.Writer.WriteAsync(item, ct); ...; channel.Writer.Complete(); // produtor
```

`ReadAllAsync` é um iterador assíncrono sobre `WaitToReadAsync` + `TryRead`; termina quando o canal é
completado e esvaziado.

## Por quê

- **Backpressure de primeira classe**: `Bounded` + `Wait` impede que produtores rápidos estourem memória; os
  modos `Drop*` oferecem políticas de perda para telemetria/tempo real.
- **`TryX` + `WaitToXAsync`**: permite drenar em lote sem custo de `await` por item (laço `while (await
  WaitToReadAsync()) while (TryRead(out x)) ...`), e sem alocação enquanto houver dados.
- **Async sem bloquear threads**: ao contrário de `BlockingCollection` (doc 07), esperar por item não segura thread.
- **Especialização por contrato** (`SingleReader`, `SingleWriter`): contratos mais fortes → implementações mais
  baratas.
- **Evitar reentrância**: `AllowSynchronousContinuations = false` impede que o produtor execute código arbitrário do
  consumidor dentro de `TryWrite` (potencial deadlock ou inversão de lock).
- **Fechamento explícito** (`Complete`) com erro opcional resolve a terminação limpa de pipelines.

## Trade-offs

- `BoundedChannel` usa **um lock** para tudo; é simples e correto, mas limita escalabilidade sob contenção
  extrema (comparado à fila lock-free do unbounded).
- Contratos de `SingleReader/SingleWriter` são **promessas do usuário**: violá-las gera comportamento incorreto.
- Continuação assíncrona por padrão custa uma ida ao pool (latência, perda de localidade);
  continuação síncrona é mais rápida, mas arriscada.
- Operações canceláveis não são pooled → cada `ReadAsync(token)` que precisa esperar aloca (a otimização só vale
  para chamadas sem token cancelável).
- `ValueTask` impõe as regras de uso único (doc 08).
- Drop modes perdem dados silenciosamente (exceto via callback `itemDropped`).
- Sem seleção múltipla (`select`) nativa entre vários canais; compõe-se com `Task.WhenAny`/`WaitToReadAsync`.
- Prioridade (`UnboundedPrioritizedChannel`) é código duplicado do unbounded e mantido "em sincronia" manualmente
  (comentário no arquivo).

## Como aplicar na sua linguagem

1. **Canais como primitiva de biblioteca (ou de linguagem)** com extremidades tipadas: `Sender`/`Receiver`.
2. **Capacidade como parâmetro**: 0 (rendezvous), N (limitado) e ilimitado, com política de cheio
   (esperar/descartar), como `BoundedChannelFullMode`.
3. **Fechamento com erro** e propagação para leitores e escritores pendentes.
4. **API em duas camadas**: `try_*` síncrono + `wait_to_*` assíncrono + `*_async` de conveniência.
5. **Futuro leve para esperas** (doc 08): objeto de operação reutilizável por leitor/escritor,
   com token/versão; **não** reutilizar quando há cancelamento.
6. **Completar continuações fora do lock** do canal, e decidir se rodam inline ou agendadas (padrão: agendadas).
7. **Especializações por cardinalidade** (SPSC, MPSC, MPMC) escolhidas pela fábrica a partir de opções.
8. **Integração com iteração assíncrona** (`for await`) e com cancelamento (doc 09).
9. **Avalie `select`** (esperar vários canais) como construção de primeira classe — o .NET não tem, outras
   linguagens (por exemplo, Go) têm; decisão de design.
10. **Invariantes explícitos** (`AssertInvariants` do `BoundedChannel`) ajudam a validar o código de filas de espera.

## Referências

- `src/libraries/System.Threading.Channels/src/System/Threading/Channels/Channel.cs`, `Channel_1.cs`, `Channel_2.cs`
- `.../Channels/ChannelReader.cs`, `ChannelWriter.cs`, `ChannelOptions.cs`, `BoundedChannelFullMode.cs`
- `.../Channels/BoundedChannel.cs`, `UnboundedChannel.cs`, `SingleConsumerUnboundedChannel.cs`
- `.../Channels/RendezvousChannel.cs`, `UnboundedPriorityChannel.cs`
- `.../Channels/AsyncOperation.cs`, `AsyncOperation.netcoreapp.cs`, `ChannelUtilities.cs`
- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentQueue.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Sources/IValueTaskSource.cs`
