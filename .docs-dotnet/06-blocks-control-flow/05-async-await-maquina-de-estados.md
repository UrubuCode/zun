# async/await — a máquina de estados gerada

No .NET existem **duas** implementações coexistindo de `async/await`:

1. **Reescrita pelo compilador de linguagem** (modelo clássico): o compilador transforma o método `async` em um tipo de *state machine* que implementa `IAsyncStateMachine`, e usa uma struct "builder" da biblioteca (`AsyncTaskMethodBuilder<T>`, etc.) para ligar tudo a `Task`. Os tipos de apoio estão neste repositório; o compilador não.
2. **Runtime-async** (novo): o próprio JIT/runtime faz a transformação em máquina de estados, a partir de métodos marcados com `MethodImplOptions.Async`. Especificação e implementação estão neste repositório.

O documento descreve ambos, deixando claro o que é confirmado por código local.

## Mecanismo no .NET

### A. Modelo clássico: state machine gerada pelo compilador

**O que é convenção do compilador (não está neste repo).** O compilador C# gera, para cada método `async`, um tipo aninhado cujo nome segue o padrão `<NomeDoMétodo>d__N`. Isso é confirmado indiretamente por `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs`, que reconhece "State machines are generated into types with names like `<OwnerMethodName>d__0`" (e, para local functions, `<<OwnerMethodName>g__Local>d`). O método original vira um stub que cria a máquina, inicializa o builder e chama `Start`. O método é anotado com `AsyncStateMachineAttribute` (`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/AsyncStateMachineAttribute.cs`, derivado de `StateMachineAttribute`, que guarda o `StateMachineType`).

**O que o runtime/biblioteca fornece (confirmado):**

- `IAsyncStateMachine` (`.../CompilerServices/IAsyncStateMachine.cs`): dois métodos, `void MoveNext()` e `void SetStateMachine(IAsyncStateMachine)`.
- `INotifyCompletion.OnCompleted(Action)` e `ICriticalNotifyCompletion.UnsafeOnCompleted(Action)`: contrato dos *awaiters*.
- `AsyncTaskMethodBuilder`, `AsyncTaskMethodBuilder<TResult>`, `AsyncValueTaskMethodBuilder[<T>]`, `PoolingAsyncValueTaskMethodBuilder[<T>]`, `AsyncVoidMethodBuilder`, `AsyncIteratorMethodBuilder`, todos em `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/`. O builder é escolhido via `AsyncMethodBuilderAttribute` (`AsyncMethodBuilderAttribute.cs`).

**Protocolo builder (API observável em `AsyncTaskMethodBuilderT.cs`):**

```
static Create()                           // linha 25 (versão não genérica): "=> default"
Start<TSM>(ref TSM sm)                    // executa sm.MoveNext() pela primeira vez, sincronamente
AwaitOnCompleted / AwaitUnsafeOnCompleted // registra continuação no awaiter
SetResult(TResult) / SetException(Exception)
SetStateMachine(IAsyncStateMachine)
Task { get; }
```

`AsyncMethodBuilderCore.Start` (`AsyncMethodBuilderCore.cs`, linha 21) mostra um uso sem rodeios de `try/finally`:

```csharp
ExecutionContext? previousExecutionCtx = currentThread._executionContext;
SynchronizationContext? previousSyncCtx = currentThread._synchronizationContext;
try { stateMachine.MoveNext(); }
finally { /* restaura SynchronizationContext e ExecutionContext alterados */ }
```

O comentário diz o motivo: desfazer mudanças de contexto feitas em `MoveNext` "para que não vazem para fora do primeiro await".

**Fluxo de um `await` que não completa de imediato** (reconstituído a partir de `AsyncTaskMethodBuilderT.cs`):

1. O `MoveNext` do compilador obtém o awaiter, vê que `IsCompleted == false`, salva o estado (campo `<>1__state`, convenção do compilador) e chama `builder.AwaitUnsafeOnCompleted(ref awaiter, ref this)`.
2. `AwaitUnsafeOnCompleted` chama `GetStateMachineBox` (linha ~188), que captura o `ExecutionContext` (`ExecutionContext.CaptureForSuspension`) e **encaixota a máquina de estados** em `AsyncStateMachineBox<TStateMachine>` (linha 415, que é também a própria `Task<TResult>`; campo `StateMachine` "mutable struct; do not make this readonly").
3. Há *fast paths* para awaiters conhecidos (`ITaskAwaiter`, `IConfiguredTaskAwaiter`, `IStateMachineBoxAwareAwaiter`): evitam alocar um delegate; o caso geral usa `awaiter.UnsafeOnCompleted(box.MoveNextAction)`. Exceções ao registrar são redirecionadas com `Task.ThrowAsync(e, targetContext: null)`.
4. O método retorna ao chamador (a `Task` já existe, incompleta).
5. Quando o awaiter completa, invoca `box.MoveNext()` (`AsyncStateMachineBox.MoveNext`, linha ~486), que restaura o `ExecutionContext` guardado (`ExecutionContext.RunInternal(context, s_callback, this)`) e chama `StateMachine.MoveNext()`; se a tarefa completou, `ClearStateUponCompletion()` limpa o estado.
6. Ao fim, o `MoveNext` do compilador chama `SetResult` (usa `Task.FromResult` se a task ainda não existia, otimização da linha ~647) ou `SetException` (`TrySetCanceled` se for `OperationCanceledException`, senão `TrySetException`).

Dois detalhes de design visíveis no código:
- **Evitar alocação no caminho síncrono.** `SetResult` com `m_task is null` cria a task já concluída; nenhuma box foi necessária.
- **Struct vs. classe.** Comentário do builder: "AsyncTaskMethodBuilder is a value type, and thus it is copied by value. Prior to being copied, one of its Task, SetResult, or SetException members must be accessed, or else the copies may end up building distinct Task instances."

**Try/catch dentro do MoveNext (convenção do compilador).** O compilador envolve o corpo num `try { ... } catch (Exception e) { state = -2; builder.SetException(e); return; }`. A razão de design é evidente do protocolo: exceções do corpo viram `Task` falhada, não propagam para o chamador do `MoveNext` (executado, por exemplo, no thread pool). A classe `Task` expõe `Task.ThrowAsync` para exceções que não têm onde ser armazenadas.

### B. Runtime-async: transformação feita pelo JIT

Especificação: `docs/design/specs/runtime-async.md`. Resumo confirmado:

- Métodos `async` são marcados `[MethodImpl(MethodImplOptions.Async)]` e retornam `Task`, `Task<T>`, `ValueTask` ou `ValueTask<T>`. Combinação com `Synchronized` é inválida; retorno byref/ref-like é inválido; `vararg` inválido.
- Suspensão através de `System.Runtime.CompilerServices.AsyncHelpers.Await(...)`, `AwaitAwaiter`, `UnsafeAwaitAwaiter` (arquivos `AsyncHelpers.cs` e `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/AsyncHelpers.CoreCLR.cs`).
- Em IL, a sequência preferida é `call <método async>` seguida de `call AsyncHelpers.Await` (`docs/design/coreclr/botr/runtime-async-codegen.md`).
- **Locals "hoisted"**: só os locals vivos através de um ponto de suspensão são preservados. Byrefs, structs ref-like e locals *pinned* não são hoisted e valem `null`/default após a suspensão.
- **Restrições**: `tail.` e `localloc` proibidos (temporário); **pontos de suspensão não podem aparecer em blocos de handler** (`catch`, `filter`, `finally`, `fault`), apenas no `try` protegido.

Implementação no JIT: `src/coreclr/jit/async.cpp` (cabeçalho do arquivo descreve as fases) e `async.h`, `asyncanalysis.cpp`:

1. Após o import, antes de inlining: para chamadas async que exigem semântica de contexto, são inseridas capturas e restaurações de `ExecutionContext`/`SynchronizationContext`, com `try/finally` quando a chamada está dentro de região try.
2. Perto do lowering: cada chamada async vira um **ponto de suspensão** + **ponto de retomada**. Na suspensão é criado um objeto **`Continuation`** contendo: todos os locals vivos, número do estado, valor de retorno (preenchido depois pelo callee), informação de exceção, ponteiro da função de retomada e flags.
3. Há um bloco de entrada que inspeciona a continuação recebida e salta para o ponto de retomada certo (`BBF_ASYNC_RESUMPTION`, `BBF_CATCH_RESUMPTION` em `block.h`, linhas 447-448).
4. O cabeçalho cita tratamento de propagação de exceção entre awaits, tipos de retorno (primitivos, refs, structs), interação com tiering/OSR e **captura otimizada pela análise de liveness**.

`runtime-async-codegen.md` acrescenta regras de ABI:
- "Wrap the body of Task and ValueTask returning functions in try/finally blocks which set/reset the ExecutionContext and SynchronizationContext."
- "If an async function is called within a try block (in the jit hasTryIndex return true), allocate space for an exception in the continuation" e codifique seu offset nas flags.
- ByRef locals não podem ser capturados na continuação.
- `ContinuationContextHandling` define `None`, `ContinueOnCapturedContext` e `ContinueOnThreadPool`.

### C. Diferenças em uma tabela

| Aspecto | Clássico (compilador) | Runtime-async (JIT) |
|---|---|---|
| Quem gera a máquina | Compilador de linguagem, em IL | JIT, em tempo de compilação nativa |
| Estado do método suspenso | Campos do tipo `<M>d__N` (struct ou classe) + box na `Task` | Objeto `Continuation` criado só ao suspender |
| Custo no caminho síncrono | Aloca/zera a máquina de estados (struct no stack), builder | Chamada direta; nada alocado se não suspende |
| `await` em `catch/finally` | Suportado (compilador reescreve) | Não suportado em handlers |
| Locals após suspensão | Hoisted como campos | Só vivos; byref/ref-like/pinned perdidos |
| Debug/stack trace | Via atributos e box | Informação de debug de async ainda "não descrita" no doc de codegen |

## Por quê

1. **Evitar threads bloqueadas.** `await` libera o thread; o estado do método precisa viver no heap enquanto suspenso. Daí a máquina de estados.
2. **Reuso de infraestrutura.** O builder liga a linguagem à `Task` sem que o compilador conheça detalhes de agendamento, contextos ou depuração.
3. **Padrão de awaiter extensível.** `INotifyCompletion`/`ICriticalNotifyCompletion` permitem que qualquer tipo seja aguardável sem herdar de nada.
4. **Evitar alocações.** Builders são structs; a box só é criada na primeira suspensão real; `ValueTask` e `PoolingAsyncValueTaskMethodBuilder` reduzem alocações adicionais.
5. **Runtime-async.** A motivação declarada na spec: "it's believed that implementation directly in the .NET runtime can provide improvements, especially in performance". O JIT conhece liveness, então salva só o necessário, e evita a máquina de estados quando nada suspende.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Máquina de estados no compilador | Independente de versão de runtime | Muito IL gerado; difícil de depurar; locais viram campos |
| Struct state machine + box na primeira suspensão | Zero alocação se completar sincronamente | Código complexo (copiar structs, `SetStateMachine`, mutabilidade) |
| `ExecutionContext` restaurado por `try/finally` em `Start` | Isola mudanças de contexto | Custo por início de método async |
| Exceção em `SetException` em vez de propagar | Semântica de `Task` | `async void` precisa de caminho especial (`AsyncVoidMethodBuilder`) |
| Runtime-async sem suspensão em handlers | JIT mais simples; EH e continuação não se misturam | Linguagem tem de reescrever `await` em `catch/finally` |
| Continuação só com locals vivos | Menos memória | Locals byref/pinned/ref-like não sobrevivem |
| Awaiters especiais com fast-path | Menos alocação | Muitos tipos "mágicos" no builder (`ITaskAwaiter`, ...) |

## Como aplicar na sua linguagem

1. **Decida onde fica a transformação.** (a) No front-end: simples de portar, mas gera muito código e você precisa de uma biblioteca de builders; (b) no backend/JIT: melhor desempenho e depuração, exige IR com suspensão/retomada e análise de liveness.
2. **Defina o protocolo de awaitable mínimo**: `is_completed`, `on_completed(callback)`, `get_result`. O .NET prova que isso basta para interoperar com qualquer fonte de eventos.
3. **Separe estado de suspensão em um objeto de "continuação"**: número do estado, locals vivos, resultado, exceção, função de retomada. Esse é exatamente o modelo do `Continuation` do JIT.
4. **Trate o caminho síncrono como o caso comum**: não aloque se o awaitable já está completo.
5. **Defina regras para handlers**: o .NET runtime-async proíbe suspender em `catch/finally`. Se você permitir, precisa de uma estratégia para salvar a exceção em voo e a posição do handler (o doc de codegen reserva espaço para uma exceção na continuação só para suspensão dentro de `try`).
6. **Defina o que acontece com referências interiores**: locals byref/ref-like/pinned não podem cruzar suspensão. Faça o verificador da sua linguagem rejeitar o uso após `await` ou zere-os como o .NET.
7. **Especifique contexto ambiente** (locale, contexto de execução, sincronização): decida se flui automaticamente e quem o restaura (`try/finally` ao redor do corpo, como no builder e no runtime-async).
8. **Exceções**: faça com que exceções do corpo virem estado "falho" do futuro/promessa, e ofereça um caminho para exceções sem destino (equivalente a `Task.ThrowAsync`).
9. **Projete cancelamento e debug cedo**: o .NET depende de atributos (`AsyncStateMachineAttribute`) e nomes dos campos ("Debugger depends on the exact name of this field", comentários no builder) para depuração.

## Apêndice: esqueleto conceitual de uma state machine (clássica)

Pseudocódigo coerente com a API do builder; os nomes de campos são convenção do compilador.

```csharp
struct M_d__0 : IAsyncStateMachine {
    public int state;                       // -1 = inicial/em execução, >=0 = ponto de retomada, -2 = concluído
    public AsyncTaskMethodBuilder<int> builder;
    public int x;                           // parâmetro/local hoisted
    private TaskAwaiter<int> awaiter;       // awaiter pendente

    public void MoveNext() {
        int result;
        try {
            switch (state) {
                case -1:
                    var t = Outro(x);
                    awaiter = t.GetAwaiter();
                    if (!awaiter.IsCompleted) {
                        state = 0;
                        builder.AwaitUnsafeOnCompleted(ref awaiter, ref this);
                        return;                         // suspende
                    }
                    goto case 0;
                case 0:
                    result = awaiter.GetResult() + 1;   // retomada
                    break;
            }
        }
        catch (Exception e) { state = -2; builder.SetException(e); return; }
        state = -2;
        builder.SetResult(result);
    }
    public void SetStateMachine(IAsyncStateMachine sm) { }
}
// stub do método original:
//   var sm = new M_d__0 { x = x, state = -1, builder = AsyncTaskMethodBuilder<int>.Create() };
//   sm.builder.Start(ref sm);
//   return sm.builder.Task;
```

Tudo acima, exceto as interfaces e o builder, é convenção do compilador. Mapeamento com o código verificado: `Create()` (`AsyncTaskMethodBuilder.cs`), `Start` (`AsyncMethodBuilderCore.cs` linha 21), `AwaitUnsafeOnCompleted`, `SetResult`/`SetException` (`AsyncTaskMethodBuilderT.cs`), `IAsyncStateMachine.MoveNext/SetStateMachine`.

## Apêndice: perguntas de design

1. A transformação é do front-end ou do backend? (Ver comparação A/B.)
2. Qual é o tipo da "promessa"? `Task` (alocada por chamada), `ValueTask` (sem alocação quando síncrono) ou um tipo próprio via `AsyncMethodBuilderAttribute`.
3. Qual o protocolo de awaiter? `IsCompleted` + `OnCompleted` + `GetResult`.
4. Onde roda a continuação? Contexto de sincronização capturado, thread pool, ou inline (`ContinuationContextHandling` no runtime-async: `None`, `ContinueOnCapturedContext`, `ContinueOnThreadPool`).
5. Como fluem contextos ambientes (`ExecutionContext`)? Capturados na suspensão e restaurados na retomada.
6. O que acontece com exceções não observadas? A `Task` guarda a exceção; `Task.ThrowAsync` serve quando não há destino.
7. Suspender dentro de `finally`/`catch` é permitido? No runtime-async não.
8. Como depurar? Atributos, nomes de campos estáveis e informação de async no debug info (ainda "não descrita" para o JIT no doc de codegen).
9. Como evitar alocação no caminho síncrono? Box só na primeira suspensão; `Task.FromResult` em `SetResult`.
10. Como evitar vazamentos de memória? `ClearStateUponCompletion` zera estado quando a box conclui.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/AsyncTaskMethodBuilderT.cs` (builder genérico, `AsyncStateMachineBox`, `SetResult`, `SetException`).
- `.../AsyncMethodBuilderCore.cs` (`Start`), `IAsyncStateMachine.cs`, `INotifyCompletion.cs`, `AsyncStateMachineAttribute.cs`, `StateMachineAttribute.cs`, `AsyncMethodBuilderAttribute.cs`, `AsyncValueTaskMethodBuilder*.cs`, `PoolingAsyncValueTaskMethodBuilder*.cs`, `AsyncVoidMethodBuilder.cs`, `AsyncHelpers.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/AsyncHelpers.CoreCLR.cs`, `RuntimeAsyncTaskContinuation.cs`.
- `docs/design/specs/runtime-async.md` (especificação), `docs/design/coreclr/botr/runtime-async-codegen.md` (responsabilidades do gerador de código), `docs/design/coreclr/jit/runtime-async-inlining.md`.
- `src/coreclr/jit/async.cpp`, `async.h`, `asyncanalysis.cpp`, `block.h` (flags `BBF_ASYNC_RESUMPTION`, `BBF_CATCH_RESUMPTION`).
- `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs` (convenção de nomes `<M>d__N`).
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/` (`ValueTask.cs`, `TaskCompletionSource.cs`).

> Nota de confiança: o formato interno do código que o compilador C# emite para `MoveNext` (campo `<>1__state`, `try/catch` envolvente) é convenção do Roslyn, não verificada neste repositório.
