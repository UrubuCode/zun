# ValueTask e otimização de alocação em async

`ValueTask`/`ValueTask<T>` existem para que operações assíncronas que frequentemente completam
sincronamente (ou cujo objeto de espera pode ser reutilizado) não aloquem uma `Task` a cada chamada.
Este documento cobre o tipo, `IValueTaskSource`, `ManualResetValueTaskSourceCore`, os builders e os caches
de `Task`.

## Mecanismo no .NET

### O struct `ValueTask` (`Threading/Tasks/ValueTask.cs`)

`[AsyncMethodBuilder(typeof(AsyncValueTaskMethodBuilder))] public readonly struct ValueTask`:

```
internal readonly object? _obj;     // null | Task | IValueTaskSource
internal readonly short   _token;
internal readonly bool    _continueOnCapturedContext;
```

Para `ValueTask<T>` há um `_result` adicional. `_obj` é a única fonte de verdade sobre "o que há aqui":

- `null` → já completou com sucesso (no `ValueTask<T>`, o resultado está no campo `_result`).
- `Task`/`Task<T>` → envolve uma task comum.
- `IValueTaskSource`/`IValueTaskSource<T>` → fonte reutilizável, identificada por `_token`.

Comentário no topo do arquivo: o código usa `Unsafe.As` para converter `_obj` sem custo e **não guarda
informação de tipo em outro campo**, porque se o `ValueTask` for copiado para um campo e lido/escrito
concorrentemente poderia ocorrer *tearing* entre `_obj` e o campo de tipo. O padrão é copiar `_obj` para um
local, testar nulo e depois o tipo.

### Regras de uso (documentadas no código)

Um `ValueTask` deve ser **aguardado diretamente uma única vez**. Não se deve:

1. Recuperar o resultado mais de uma vez — a fonte pode ser reciclada quando o resultado é consumido.
2. Adicionar múltiplas continuações.
3. Usar depois de operações posteriores que podem invalidá-lo (salvo documentação em contrário).

Para algo mais complicado, converte-se com `AsTask()` (que devolve a task existente ou "fabrica" uma via
`GetTaskForValueTaskSource`/`ValueTaskSourceAsTask`) ou `Preserve()` (devolve um `ValueTask` utilizável a qualquer
momento, `new ValueTask(AsTask())`).

### `IValueTaskSource` (`Tasks/Sources/IValueTaskSource.cs`)

Contrato para implementar o "lado produtor" sem alocar `Task`:

```
ValueTaskSourceStatus GetStatus(short token);   // Pending | Succeeded | Faulted | Canceled
void OnCompleted(Action<object?> continuation, object? state, short token,
                 ValueTaskSourceOnCompletedFlags flags);
void GetResult(short token);   // (e a versão genérica retorna TResult)
```

`ValueTaskSourceOnCompletedFlags`: `None`, `UseSchedulingContext` (capturar `SynchronizationContext`/
`TaskScheduler` atual e usar na retomada) e `FlowExecutionContext` (capturar `ExecutionContext`).
O **token** (`short`) é um "número de versão" opaco: protege contra uso de um `ValueTask` obsoleto depois que
a fonte foi reutilizada.

### `ManualResetValueTaskSourceCore<TResult>` (struct)

Implementação reutilizável do contrato, para compor dentro de outro objeto (ex.: um canal ou um stream):

- Campos: `_continuation` (Action), `_continuationState`, `_capturedContext`, `_error`
  (`ExceptionDispatchInfo`), `_result`, `_version`, `_runContinuationsAsynchronously`.
- `Reset()` incrementa `_version` e limpa tudo → permite reuso; tokens antigos passam a falhar em `ValidateToken`.
- `SetResult`/`SetException` → `SignalCompletion`, que faz `Interlocked.Exchange(ref _continuation,
  s_sentinel)`: se havia continuação registrada, é invocada (inline, via `ExecutionContext`/scheduler capturado,
  ou assincronamente se `RunContinuationsAsynchronously`).
- `OnCompleted`: registra a continuação com `Interlocked.CompareExchange(ref _continuation, continuation, null)`; se
  a fonte já tinha completado (viu o sentinela), a continuação é **enfileirada** ("sempre enfileiramos aqui,
  mesmo que !RunContinuationsAsynchronously"). Captura `ExecutionContext.Capture()` e, conforme flags,
  `SynchronizationContext.Current`/`TaskScheduler.Current` (`CapturedSchedulerAndExecutionContext`).
- `GetStatus` mapeia `OperationCanceledException` em `Canceled`.

### Builders

`CompilerServices/`:

1. **`AsyncValueTaskMethodBuilder<TResult>`**: struct que guarda `m_task` (`Task<TResult>?`), `_result` e usa
   `s_syncSuccessSentinel` (uma `Task<TResult>` estática). Se o método `async ValueTask<T>` completa sem
   suspender, `SetResult` guarda o valor em `_result` e `Task` devolve `new ValueTask<TResult>(_result)` — **zero
   alocação**. Se suspende, delega ao `AsyncTaskMethodBuilder<TResult>` (que aloca a `AsyncStateMachineBox`,
   uma `Task<TResult>`) e devolve `new ValueTask<TResult>(task)`.
2. **`PoolingAsyncValueTaskMethodBuilder[<TResult>]`** (opt-in via `[AsyncMethodBuilder(typeof(Pooling...))]`
   no método): a caixa da máquina de estados (`StateMachineBox<TStateMachine>`) **implementa
   `IValueTaskSource<TResult>`, `IValueTaskSource`, `IAsyncStateMachineBox` e `IThreadPoolWorkItem`**, e é
   reciclada:
   - Cache **por thread** (`[ThreadStatic] t_tlsCache`, guarda no máximo uma caixa).
   - Cache **por core** (`s_perCoreCache`, `PaddedReference[Environment.ProcessorCount]`, indexado por
     `Thread.GetCurrentProcessorId() % ProcessorCount`, com padding de cache-line contra false sharing).
   - `RentFromCache`: tenta TLS → senão `Interlocked.Exchange(ref slot, null)` do slot do core → senão `new`.
   - `ReturnToCache`: `ClearStateUponCompletion()` + `_valueTaskSource.Reset()` (muda a versão), grava no TLS se
     livre, senão no slot do core (com `Volatile.Write`; uma corrida benigna pode sobrescrever).
   - Consequência: o `ValueTask` devolvido **só pode ser consumido uma vez**; o consumo pode devolver a
     caixa ao pool.

### Outros caches contra alocação

- `TaskCache` (`Tasks/TaskCache.cs`): `s_trueTask`, `s_falseTask` (`Task<bool>`) e `s_int32Tasks`
  (`Task<int>` pré-alocadas para um intervalo pequeno), usadas por `Task.FromResult` e pelo builder de `Task<T>`.
- `Task.CompletedTask` e `Task.FromResult`.
- O objeto único: no `AsyncTaskMethodBuilderT`, a mesma alocação serve de Task, estado e callback (doc 03).
- No `Channels`: `AsyncOperation<T>` implementa `IValueTaskSource<T>` e pode ser "pooled" por leitor/escritor
  (`pooled: true`, "singleton" por reader/writer quando não há cancelamento) — doc 10.
- `SemaphoreSlim.WaitAsync` e outras APIs ainda retornam `Task` (não `ValueTask`) em muitos casos.

### Awaiters

`ValueTaskAwaiter.cs`, `ConfiguredValueTaskAwaitable.cs`: `IsCompleted` checa `_obj` sem alocar; o
awaiter implementa `IStateMachineBoxAwareAwaiter`, permitindo ao builder registrar a caixa diretamente em
`IValueTaskSource.OnCompleted` (sem criar delegate extra) — ramo visto em `AwaitUnsafeOnCompleted` (doc 03).
Se a fonte lançar ao registrar a continuação, o erro é re-lançado de forma assíncrona (`Task.ThrowAsync`) porque
o estado da máquina de estados já pode ter outro fluxo ativo.

## Por quê

- **Caminho síncrono quente**: leituras de buffer, caches, canais com item disponível completam sem esperar.
  Alocar uma `Task` por chamada gera pressão no GC desproporcional ao trabalho.
- **Reutilização**: fontes como sockets, canais ou streams podem manter *um* objeto de operação por
  instância e reutilizá-lo para cada operação, com `token`/versão detectando uso inválido.
- **Compatibilidade**: ser `await`-able e conversível em `Task` evita fragmentar o ecossistema.
- **Struct de uma palavra + token**: `_obj` + `_token` + flag cabem em poucos bytes; `Unsafe.As` e o JIT removem
  checagens de tipo.

## Trade-offs

- **API frágil**: aguardar duas vezes, guardar em campo e esperar depois, ou usar `.Result` antes de completar
  são erros de uso com comportamento indefinido quando há reciclagem (o próprio comentário no código usa
  "undefined behavior").
- **Struct maior** que referência (2 palavras + flags, mais `_result` em `ValueTask<T>`): copiá-lo é
  mais caro; guardá-lo em muitos campos aumenta memória.
- **Pooling não é de graça**: caches por thread/core têm custo de código, `Interlocked.Exchange`, e retêm memória
  (a caixa guarda o tipo da máquina de estados genérica — mais instanciações genéricas = mais código, e o
  comentário no código evita lambdas para não aumentar o tamanho em AOT).
- **Corridas benignas** no cache por core ("podemos sobrescrever o que chegou no meio-tempo").
- **Debug**: tokens e versões dificultam inspeção; o runtime precisa de campos especiais para o depurador.
- O padrão **opt-in** de pooling existe porque reciclar caixas muda a semântica visível ao chamador
  (um `ValueTask` pooled não pode ser aguardado duas vezes, ao contrário de uma `Task`).
- Quem implementa `IValueTaskSource` precisa tratar concorrência entre `SetResult` e `OnCompleted`
  (a lógica de `Interlocked` do `ManualResetValueTaskSourceCore` existe para isso).

## Como aplicar na sua linguagem

1. **Dois tipos de futuro**: um "pesado" (heap, compartilhável, múltiplos awaiters) e um "leve" (valor, uma
   única espera). Se tiver só um, faça o compilador otimizar o caminho síncrono sem alocar.
2. **Representação do leve**: uma referência + token de versão + (opcional) valor inline. Uma única palavra de
   tipo-estado evita *tearing* (copie sempre o campo para local antes de inspecionar).
3. **Contrato para fontes reutilizáveis** (`status/on_completed/get_result` com token) e uma implementação-padrão
   (`ManualResetValueTaskSourceCore`) que lide com a corrida set-result × registrar-continuação via CAS e sentinela.
4. **Versionamento**: incremente a versão em `reset`; valide o token em toda operação. Considere checar em
   builds de debug com falha determinística.
5. **Pooling opt-in** e por atributo/anotação no método, com cache por thread + por core; documente claramente
   a regra "consumir uma vez".
6. **Considere impor a regra de uso por tipo linear/afim** (consumo único checado pelo compilador) em vez de
   por convenção — o .NET não faz isso; é uma melhoria possível numa linguagem nova.
7. **Pré-aloque futuros comuns** (true, false, 0, unit).
8. **Retorno síncrono sem alocar** deve ser o padrão do compilador de funções async (como
   `AsyncValueTaskMethodBuilder` com `s_syncSuccessSentinel`).
9. **Cuidado com instanciação genérica/monomorfização**: pools genéricos multiplicam código; o .NET evita
   lambdas nesses pontos por tamanho em AOT.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/ValueTask.cs`
- `.../Threading/Tasks/Sources/IValueTaskSource.cs`
- `.../Threading/Tasks/Sources/ManualResetValueTaskSourceCore.cs`
- `.../Threading/Tasks/TaskCache.cs`
- `.../Runtime/CompilerServices/AsyncValueTaskMethodBuilder.cs`, `AsyncValueTaskMethodBuilderT.cs`
- `.../Runtime/CompilerServices/PoolingAsyncValueTaskMethodBuilder.cs`, `PoolingAsyncValueTaskMethodBuilderT.cs`
- `.../Runtime/CompilerServices/ValueTaskAwaiter.cs`, `ConfiguredValueTaskAwaitable.cs`
- `.../Runtime/CompilerServices/AsyncTaskMethodBuilderT.cs`
- `src/libraries/System.Threading.Channels/src/System/Threading/Channels/AsyncOperation.cs`
