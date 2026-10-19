# async/await: scheduling e SynchronizationContext

Como `async/await` é implementado no .NET: a transformação do compilador em máquina de estados, o padrão
"builder + awaiter", como a continuação é agendada (contexto de sincronização, scheduler, pool) e a
proposta "runtime async" que move a transformação para dentro do runtime.

## Mecanismo no .NET

### O padrão: awaitable, awaiter e builder

`await expr` não é mágico: o compilador C# chama membros por convenção. Os tipos de suporte vivem em
`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/`:

- **Awaiter** (`TaskAwaiter.cs`, `ValueTaskAwaiter.cs`): expõe `IsCompleted`, `OnCompleted`/`UnsafeOnCompleted`
  (interfaces `INotifyCompletion` e `ICriticalNotifyCompletion`) e `GetResult`.
- **Builder** (`AsyncTaskMethodBuilder.cs`, `AsyncTaskMethodBuilderT.cs`, `AsyncValueTaskMethodBuilder*.cs`,
  `PoolingAsyncValueTaskMethodBuilder*.cs`, `AsyncVoidMethodBuilder.cs`, `AsyncIteratorMethodBuilder.cs`):
  cria o resultado do método (a Task), recebe `SetResult`/`SetException`, e agenda a retomada com
  `AwaitOnCompleted`/`AwaitUnsafeOnCompleted`. O tipo do builder é escolhido pelo atributo
  `[AsyncMethodBuilder(typeof(...))]` (`AsyncMethodBuilderAttribute.cs`) — é o ponto de extensão que permite
  `ValueTask` e builders customizados.
- **Máquina de estados** (`IAsyncStateMachine.cs`): struct gerada pelo compilador com `MoveNext()`; variáveis
  locais que cruzam um `await` viram campos.

### Caminho rápido: completou sincronamente

O código gerado testa `awaiter.IsCompleted`; se verdadeiro, segue sem suspender e sem alocar a máquina de
estados no heap (ela vive na pilha enquanto o método não suspende).

### Suspensão: a "caixa" no heap

Na primeira suspensão, `GetStateMachineBox` (`AsyncTaskMethodBuilderT.cs`) move a máquina de estados para
um `AsyncStateMachineBox<TStateMachine>`, que **é uma `Task<TResult>`** e implementa `IAsyncStateMachineBox`:

- A mesma alocação serve de (1) Task retornada ao chamador, (2) dono da struct de estado
  (`public TStateMachine? StateMachine; // mutable struct; do not make this readonly`), (3) dono do
  delegate `MoveNextAction` (criado preguiçosamente em `m_action`) e (4) dono do `ExecutionContext`
  capturado — guardado no campo `m_stateObject` da Task base, marcado com `HiddenState` para não vazar por
  `AsyncState`.
- `ExecutionContext.CaptureForSuspension(Thread.CurrentThread)` é chamado ao criar a caixa.
- Na segunda suspensão em diante a caixa existente é reutilizada (o código checa se `taskField` já é a
  `AsyncStateMachineBox` tipada).

### Despacho do `await` conforme o tipo do awaiter

`AwaitUnsafeOnCompleted<TAwaiter>(ref awaiter, IAsyncStateMachineBox box)` testa, na ordem:
`ITaskAwaiter`, `IConfiguredTaskAwaiter`, `IStateMachineBoxAwareAwaiter` (ValueTask), e por fim o caso
genérico `awaiter.UnsafeOnCompleted(box.MoveNextAction)`. Os testes `null != (object?)default(TAwaiter)`
permitem ao JIT eliminar os ramos quando `TAwaiter` é tipo referência, e o uso de `Unsafe.As` evita
cópias. Para `Task`, a caixa é registrada diretamente como continuação (sem criar delegate): `Task.UnsafeSetContinuationForAwait`.
Exceções ao registrar são relançadas com `Task.ThrowAsync` para não voltarem para dentro da máquina
de estados num ponto onde ela não consegue tratá-las.

### Onde a continuação roda: `SetContinuationForAwait`

`Task.cs` (~linhas 2640-2735), com `continueOnCapturedContext` verdadeiro:

1. Se `SynchronizationContext.Current` existe **e seu tipo não é exatamente `SynchronizationContext`** →
   `SynchronizationContextAwaitTaskContinuation`. (Comentário: o tipo base é considerado equivalente a "sem
   contexto", para evitar posts desnecessários.)
2. Senão, se `TaskScheduler.InternalCurrent` existe e não é o `Default` → `TaskSchedulerAwaitTaskContinuation`.
3. Senão → a continuação "crua" (a própria caixa/Action) sem wrapper.

`ConfigureAwait(false)` desliga os passos 1-2. `ConfigureAwaitOptions` (`ConfigureAwaitOptions.cs`):
`None=0`, `ContinueOnCapturedContext=1`, `SuppressThrowing=2`, `ForceYielding=4`
(`ForceYielding` faz `IsCompleted` devolver falso para forçar uma ida ao scheduler).

### `SynchronizationContext`

`SynchronizationContext.cs`: classe com `Post` (assíncrono), `Send` (síncrono), `OperationStarted/Completed`,
`CreateCopy`, `Wait`. `Current` lê `Thread.CurrentThread._synchronizationContext` — ou seja, é estado
**por thread**. O comportamento default de `Post` é `ThreadPool.QueueUserWorkItem(..., preferLocal: false)`;
`Send` apenas invoca o delegate. Frameworks de UI sobrescrevem para marshalling à thread de UI.

`SynchronizationContextAwaitTaskContinuation.Run` (`TaskContinuation.cs` ~linha 364): se inlining é permitido
**e** `m_syncContext == SynchronizationContext.Current`, executa na hora; senão `Post`.

### Inline versus fila ao completar

Ao completar uma Task, `RunContinuations` usa `canInlineContinuations` (sem `RunContinuationsAsynchronously`
+ pilha suficiente). `AwaitTaskContinuation.RunOrScheduleAction` roda inline ou enfileira no ThreadPool via
`ThreadPool.UnsafeQueueUserWorkItemInternal(box, preferLocal: true)` (linhas ~598, ~808, ~842 de
`TaskContinuation.cs`) — `preferLocal: true` joga a retomada na fila local do worker atual (veja doc 12).

### `ExecutionContext` e fluxo de estado

Cada retomada passa por `ExecutionContext.RunInternal(...)` (comentário: "extremamente quente, usado por
todo await, execução do ThreadPool, etc."), que restaura o contexto capturado e depois o anterior. Variantes
`Unsafe*` (`UnsafeOnCompleted`) **não** capturam o `ExecutionContext` porque a própria caixa já o capturou
uma vez. Mais no doc 11.

### Instrumentação

`AsyncInstrumentation`, `AsyncProfiler*`, `AsyncStateMachineDispatcher`: o runtime pode envolver caixas
em dispatchers para eventos ETW/depurador. Não é essencial ao design, mas mostra que o custo de observabilidade
é pago apenas quando ligado (`IsActive`).

### Runtime async (em evolução)

`docs/design/specs/runtime-async.md` é um rascunho de emendas ao ECMA-335: métodos marcados com
`[MethodImpl(MethodImplOptions.Async)]` (flag `0x2000` em `MethodImplAttributes`) que retornam `Task`,
`ValueTask`, `Task<T>` ou `ValueTask<T>` passam a ser transformados pelo **runtime/JIT** em vez do compilador.
Pontos confirmados no documento:

- Suspensão por chamadas a `System.Runtime.CompilerServices.AsyncHelpers` (`Await(Task)`, `Await(ValueTask)`,
  `AwaitAwaiter`, `UnsafeAwaitAwaiter`, versões `ConfiguredTaskAwaitable`).
- Locais usados após a suspensão são "hoisted"; **byrefs não podem ser hoisted**; locais "pinned" viram `null`.
- Restrições: sem prefixo `tail`, sem `localloc`; suspensões **não** podem ocorrer em blocos
  `catch/filter/finally/fault` (permitidas no `try`); não combina com `Synchronized`, retornos byref/ref-like, vararg.
- `docs/design/coreclr/botr/runtime-async-codegen.md` diz que o gerador de código deve envolver o corpo em
  `try/finally` que define/restaura `ExecutionContext` e `SynchronizationContext`, e há "thunks" que
  permitem interoperar com chamadas normais. `Task.cs` já contém `RuntimeAsyncTaskContinuation`, tratado
  em `RunContinuations` com regra própria (inline se só há uma continuação, paralelo se várias).
- `AsyncHelpers*.cs` em `CompilerServices` (variantes Browser/NonBrowser/Wasi) implementam o suporte.

## Por quê

- **Código sequencial sobre I/O não bloqueante**: threads não ficam presas esperando I/O; uma thread
  atende muitas operações (essencial para servidores).
- **Padrão por convenção (builder/awaiter)** em vez de interface fixa: permite tipos de retorno
  customizados e otimizações (ValueTask, pooling) sem mudar a linguagem.
- **Custo mínimo no caminho síncrono**: `IsCompleted` evita alocação e agendamento.
- **Uma alocação por método assíncrono suspenso** (a caixa que é também Task + estado + contexto).
- **Contexto de sincronização** existe para afinidade de thread (UI): a retomada volta à thread correta
  sem o usuário escrever marshalling.
- **Runtime async**: remover geração de código duplicada em todas as linguagens e permitir otimizações
  (o texto do spec diz acreditar que a implementação no runtime pode trazer melhorias — o documento é um rascunho).

## Trade-offs

- **"Colorização" de funções**: métodos async contaminam chamadores; misturar sync/async leva a
  sync-over-async (`Task.Wait`) que bloqueia threads do pool e pode dar deadlock com um `SynchronizationContext`
  de thread única.
- **Contexto implícito**: capturar `SynchronizationContext`/`TaskScheduler` por padrão é conveniente para UI,
  mas surpreende em bibliotecas; daí `ConfigureAwait(false)` e `ConfigureAwaitOptions`.
- **Complexidade do compilador**: transformação em máquina de estados, hoisting, tratamento de exceções.
- **Debugging**: pilhas de chamada fragmentadas; exige suporte de ferramentas (a infraestrutura de
  dispatchers/ETW).
- **Alocação**: ainda uma alocação por método que suspende (mitigada por ValueTask/pooling, doc 08).
- **Runtime async**: restrições (sem suspensão em `catch/finally`, sem byref across await), acopla o formato
  do IL à versão do runtime.
- **Inline de continuações**: ganho de latência, risco de reentrância e estouro de pilha (protegido por
  `TryEnsureSufficientExecutionStack`).

## Como aplicar na sua linguagem

1. **Decida cedo: stackful (corrotinas com pilha própria/green threads) ou stackless (máquina de estados).**
   O .NET escolheu stackless; isso torna cada `await` visível no tipo e permite alocar só quando suspende.
2. **Faça o protocolo extensível**: um "awaitable" com `isCompleted / onCompleted / getResult` e um
   "builder" selecionável por atributo. Isso dá a você ValueTask-like e pooling depois, sem mudar a sintaxe.
3. **Caminho síncrono sem alocação** (checar `isCompleted` antes de suspender).
4. **Unifique o objeto**: o resultado do método async, a máquina de estados e os callbacks podem ser o
   mesmo objeto (como `AsyncStateMachineBox`).
5. **Defina o contexto de retomada explicitamente.** Se mantiver algo como `SynchronizationContext`,
   documente que "contexto base = sem contexto" e ofereça um modo para desligar (equivalente a ConfigureAwait).
   Considere o inverso: **não capturar por padrão** e exigir opt-in — evita a classe de bugs de deadlock.
6. **Proteja contra estouro de pilha** em continuações síncronas encadeadas, ou sempre agende.
7. **Considere mover a transformação para o backend** (JIT/AOT) como o "runtime async", para que
   todas as linguagens compartilhem; leve em conta as restrições já descobertas (byref, pinned, handlers
   de exceção, `localloc`).
8. **Plano de depuração desde o início**: identificar a "pilha lógica" de awaits.
9. **Erros no registro de continuação** não devem voltar para dentro da máquina de estados (padrão `ThrowAsync`).

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/AsyncTaskMethodBuilderT.cs`
- `.../CompilerServices/AsyncTaskMethodBuilder.cs`, `AsyncMethodBuilderAttribute.cs`, `IAsyncStateMachine.cs`, `IAsyncStateMachineBox.cs`
- `.../CompilerServices/TaskAwaiter.cs`, `ValueTaskAwaiter.cs`, `AsyncHelpers.cs`, `AsyncInstrumentation.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/SynchronizationContext.cs`
- `.../Threading/ExecutionContext.cs`
- `.../Threading/Tasks/Task.cs` (`SetContinuationForAwait`, `RunContinuations`)
- `.../Threading/Tasks/TaskContinuation.cs`, `.../Tasks/ConfigureAwaitOptions.cs`
- `docs/design/specs/runtime-async.md`
- `docs/design/coreclr/botr/runtime-async-codegen.md`
- `docs/design/coreclr/jit/runtime-async-inlining.md`
