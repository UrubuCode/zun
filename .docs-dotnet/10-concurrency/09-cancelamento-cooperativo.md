# Cancelamento cooperativo (CancellationToken)

O .NET separa **quem pede o cancelamento** (`CancellationTokenSource`) de **quem observa** (`CancellationToken`),
e define o cancelamento como cooperativo: nada é interrompido à força; o código consulta o token, registra
callbacks, ou passa o token adiante.

## Mecanismo no .NET

### Duas peças

`src/libraries/System.Private.CoreLib/src/System/Threading/`:

- **`CancellationToken`** (`CancellationToken.cs`): `readonly struct` cujo único campo de dados é
  `private readonly CancellationTokenSource? _source`. Todos os membros são thread-safe. `default(CancellationToken)`
  tem `_source == null` e nunca cancela (`CanBeCanceled == false`), o que permite ao código otimizar
  (`CanBeCancelled` falso → sem registrar nada). API: `IsCancellationRequested`, `CanBeCanceled`,
  `ThrowIfCancellationRequested()` (lança `OperationCanceledException(SR.OperationCanceled, this)`),
  `Register(...)` e `UnsafeRegister(...)`, `WaitHandle`.
- **`CancellationTokenSource`** (`CancellationTokenSource.cs`): o "dono". `Cancel()`, `CancelAfter(delay)`,
  construtor com timeout, `CreateLinkedTokenSource(...)`, `TryReset()`, `Dispose()`.
- **`CancellationTokenRegistration`**: o recibo de `Register`; `Dispose()` cancela o registro e, se o callback
  estiver executando em outra thread, **espera sua conclusão** (`WaitForCallbackToComplete(id)`) — há também
  `DisposeAsync` (`WaitForCallbackToCompleteAsync`). `Unregister()` remove sem esperar.

### Estado do source

`private volatile States _state` com três valores: `NotCanceledState = 0`, `NotifyingState = 1`,
`NotifyingCompleteState = 2`. Transição de pedido: `Interlocked.CompareExchange(ref _state, NotifyingState,
NotCanceledState)` — apenas o primeiro chamador a vence, e faz o trabalho extra: descarta o timer, sinaliza o
evento de kernel (se criado preguiçosamente por `WaitHandle`) e executa callbacks. `IsCancellationRequested` é
`_state != NotCanceledState` (já verdadeiro durante a notificação); `IsCancellationCompleted` é
`_state == NotifyingCompleteState`. Existem singletons estáticos `s_canceledSource` e `s_neverCanceledSource`.

### Registro de callbacks

- Campo `Registrations? _registrations` **criado sob demanda** (comentário: manter o CTS pequeno quando todas as
  operações completam sincronamente ou só fazem polling). Publicado com `Interlocked.CompareExchange`.
- `Registrations` contém lista duplamente ligada de `CallbackNode`, `FreeNodeList` (lista de nós livres para
  reuso, para reduzir alocação), `NextAvailableId`, `ExecutingCallbackId` e `ThreadIDExecutingCallbacks`, protegidos por
  um spinlock curto (`EnterLock/ExitLock`; comentário: o código evita alocar nós enquanto segura o lock "para não
  fazer trabalho de GC arbitrário sob o lock, que queremos manter apertado — poucas atribuições").
- `Register` quando **já cancelado**: executa o callback imediatamente na thread chamadora e devolve registro vazio.
  Se o cancelamento ocorre durante o `Register`, tenta desfazer o registro (`Unregister(id, node)`); se não
  conseguir (a thread de cancelamento já pegou o nó), devolve o registro para que o chamador possa esperar via `Dispose`.
- Cada nó guarda o `ExecutionContext` e o `SynchronizationContext` **capturados** no registro. `Register`
  (seguro) captura; `UnsafeRegister` não captura `ExecutionContext` (mais barato, para código de infraestrutura).

### Execução dos callbacks (`ExecuteCallbackHandlers`)

1. `Interlocked.Exchange(ref _registrations, null)`; se nulo, pula (corridas com `Register` verão
   `IsCancellationRequested` e executam sozinhas).
2. Ordem **LIFO** — "deepest first": filhos cancelam antes dos pais (comentário).
3. Cada nó é **removido antes de executar**, para que desregistros posteriores continuem efetivos
   enquanto outros callbacks rodam; `ExecutingCallbackId` é publicado sob o lock para que `Dispose` do registro saiba se
   precisa esperar.
4. Se o nó tem `SynchronizationContext`, o callback roda via `syncContext.Send` (síncrono) no contexto; senão na
   thread que chamou `Cancel`.
5. Exceções: se `throwOnFirstException` é falso (padrão de `Cancel()`), coletadas e lançadas ao final como
   `AggregateException`; caso contrário a primeira propaga.
6. Ao final `_state = NotifyingCompleteState` e `Interlocked.Exchange(ref ExecutingCallbackId, 0)` (barreira
   explícita "para segurança").

### Timers, links e reset

- `CancelAfter`/construtor com delay: um `ITimer` (`TimerQueueTimer` ou do `TimeProvider`) criado preguiçosamente via
  CAS em `_timer`. `InitializeWithTimer`: se o delay é zero cancela na hora. Cria o timer com
  `ExecutionContext.SuppressFlow()` quando via `TimeProvider`.
- `CreateLinkedTokenSource(token1, token2)` → `Linked1/Linked2/LinkedNCancellationTokenSource`: registram-se
  nos tokens de origem (`UnsafeRegister`) e cancelam o source ligado; só alocam se algum token `CanBeCanceled`.
- `TryReset()`: permite reutilizar o source se não foi cancelado (e o timer não disparou); documentado como seguro
  só para o dono único e **não** concorrente com cancelamento.
- `Dispose()` libera timer/evento; admite chamada concorrente com `Register` (retorna registro vazio).

### Integração com o resto

- `Task`: `Task.Run(action, token)`, `TaskCompletionSource`, `Task.Delay(…, token)` registram o token; o
  `ContingentProperties` guarda `m_cancellationToken` e `m_cancellationRegistration` (doc 02). Convenção:
  cancelamento observado vira `TaskStatus.Canceled` via `OperationCanceledException` com o mesmo token.
- `SemaphoreSlim.WaitAsync(CancellationToken)`, `Channel<T>.Reader.ReadAsync(ct)` (a operação cancelável registra
  o token com `UnsafeRegister` e não é "pooled": `Debug.Assert(!_pooled, "Cancelable operations can't be pooled")`,
  `AsyncOperation.cs`), `Parallel` (`ParallelOptions.CancellationToken`), `PeriodicTimer`, I/O.
- `ConfiguredCancelableAsyncEnumerable` e `[EnumeratorCancellation]` para `IAsyncEnumerable`.

### Padrão de uso típico

```
using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(5));   // timeout
await DoWorkAsync(cts.Token);                       // repassa o token
// dentro de DoWorkAsync:
//   token.ThrowIfCancellationRequested();          // polling cooperativo
//   using var reg = token.Register(() => socket.Close());   // acorda uma operação bloqueada
```

Três formas de cooperar: (1) **polling** (`IsCancellationRequested`/`ThrowIfCancellationRequested`),
(2) **callback** (`Register` para interromper uma operação de I/O pendente) e (3) **espera** (`WaitHandle` ou
passar o token a APIs que o aceitam). A convenção é que a operação cancelada termine com
`OperationCanceledException` associada ao mesmo token, o que a `Task` traduz em estado `Canceled`.

## Por quê

- **Cancelamento forçado de threads é inseguro** (deixa locks e invariantes quebrados). O .NET tem
  `ThreadAbortException` apenas como tipo legado (arquivo existe em `Threading/`), mas o modelo moderno é
  cooperativo.
- **Separar dono e observador** dá segurança por tipo: um método que recebe `CancellationToken` não pode
  cancelar o chamador; token como struct de uma referência é barato de passar por toda a pilha de chamadas.
- **Composição**: tokens ligados, timeouts e cancelamento por múltiplas fontes sem mudar a API dos métodos.
- **Custo proporcional ao uso**: `Registrations`, timer e evento são lazily criados; `default` token não custa nada.
- **Garantias de callback**: `Dispose` do registro espera callback em andamento → o chamador sabe que após
  `Dispose` o callback não está mais rodando (evita usar recursos já liberados).

## Trade-offs

- **Cooperativo = depende da boa vontade**: código que não verifica o token não para; cancelar não interrompe
  chamadas bloqueantes de SO por si só (precisa de registro que acorde a operação).
- **Passagem manual do token por toda a API**: ruído sintático ("token em todo método"); esquecer de repassá-lo
  quebra a cadeia.
- **Callbacks rodam na thread de `Cancel()`**: podem demorar, lançar, ou causar reentrância; por isso as
  exceções são agregadas e há opção `throwOnFirstException`.
- **Possível deadlock** em `Dispose()` do registro se o callback espera algo que o chamador do `Dispose` segura
  (o código tenta mitigar: se o `Dispose` é chamado da própria thread de callbacks não espera —
  `ThreadIDExecutingCallbacks`).
- **Custo de captura de contexto** em `Register`; `UnsafeRegister` evita, mas com risco de perder
  `AsyncLocal`/identidade.
- **Vazamentos** se registros não são descartados (cada registro mantém o callback vivo enquanto o source viver).
- `TryReset` tem restrições de uso sutis.
- Cancelamento como **exceção** (`OperationCanceledException`) é caro no caminho de erro e mistura controle de
  fluxo com erros; há APIs de `Try` e `SuppressThrowing` (`ConfigureAwaitOptions`) para evitar exceções.

## Como aplicar na sua linguagem

1. **Token como valor leve, source como objeto**: um `Token` com ponteiro nulo representa "nunca cancela" e é
   o valor padrão.
2. **Estado em um inteiro com CAS** e três fases (não cancelado → notificando → concluído) para distinguir
   "pedido" de "callbacks terminados".
3. **Registro de callbacks lazy**, lista com reaproveitamento de nós, spinlock mínimo; callbacks LIFO.
4. **Contrato claro sobre onde os callbacks rodam** (thread do cancelador ou contexto capturado) e sobre exceções
   (agregar).
5. **`Dispose` de registro bloqueia/espera** callback em execução (com defesa contra auto-espera).
6. **Integre com o sistema de tarefas** desde o início: estado "cancelada" distinto de "falhou", e propagação do
   token ao criar tarefas/aguardar.
7. **Considere cancelamento estruturado** (escopo que cancela filhos automaticamente) como complemento ou
   substituto da passagem manual do token — decisão de design da sua linguagem (o .NET não oferece isso nativamente).
8. **Timeouts como token** (`cancel_after`), reutilizando a infraestrutura de timers.
9. **Verificação barata e rápida no caminho quente** (`ThrowIfCancellationRequested` separa o `throw` em método
   próprio para permitir inlining da checagem).
10. **Evite capturar contexto por padrão em primitivas de infraestrutura**; ofereça variante "unsafe".

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/CancellationToken.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/CancellationTokenSource.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/CancellationTokenRegistration.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Timer.cs`, `TimerQueue.*.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Task.cs` (`ContingentProperties`)
- `src/libraries/System.Private.CoreLib/src/System/Threading/SemaphoreSlim.cs`
- `src/libraries/System.Threading.Channels/src/System/Threading/Channels/AsyncOperation.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ConfiguredCancelableAsyncEnumerable.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/ThreadAbortException.cs`
