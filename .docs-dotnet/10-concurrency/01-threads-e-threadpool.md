# Modelo de threads e o ThreadPool

Este documento descreve como o .NET modela threads gerenciadas e como o ThreadPool
(implementação "portable" em C#) decide quantas threads existem, quando criá-las e como
despachar trabalho. O objetivo é extrair decisões de design reaproveitáveis para uma nova linguagem.

## Mecanismo no .NET

### Threads gerenciadas versus threads nativas

O documento BOTR `docs/design/coreclr/botr/threading.md` define que código gerenciado roda em
"threads gerenciadas", distintas das threads do SO:

- Uma thread gerenciada pode existir sem estar associada a uma thread nativa (por exemplo, criada com
  `new Thread` mas ainda não iniciada). Em princípio poderia migrar entre threads nativas, mas o texto
  diz que o CLR atualmente não suporta isso.
- A API pública esconde detalhes da thread nativa de propósito: não expõe o TLS do SO, e sim variáveis
  "thread-static"; não expõe o thread ID do SO, e sim um "managed thread ID" gerado de forma independente.
- Threads gerenciadas precisam de infraestrutura extra: o CLR precisa conseguir enumerar referências GC
  na pilha de cada thread, então precisa "suspender" cada thread em pontos seguros (safe points). Uma GC só
  prossegue quando todas as threads gerenciadas estão em modo "preemptive".
- Todas as `Thread` nativas ficam num `ThreadStore` (lista global protegida por `ThreadStoreLock`).
- Uma thread nativa pode virar gerenciada por 4 caminhos: `Thread.Start`, criação direta pelo CLR
  (threads "especiais"), reverse P/Invoke/COM, ou o processo iniciar `Main`.

Do lado C#, `System.Private.CoreLib/src/System/Threading/Thread.cs` e arquivos parciais por plataforma
(`Thread.Windows.cs`, `Thread.Unix.cs`, `Thread.Multithreading.cs`, `Thread.SingleThreading.cs`,
`Thread.Wasi.cs`) mostram que o suporte a multithreading é uma feature de build: há variantes
single-thread (WASM/WASI) onde `RuntimeFeature.IsMultithreadingSupported` é falso
(visto em `Tasks/ThreadPoolTaskScheduler.cs`, que só cria thread dedicada para `LongRunning` quando isso é verdadeiro).

### O ThreadPool: visão geral

A implementação gerenciada é `PortableThreadPool` (arquivos `PortableThreadPool*.cs` em
`src/libraries/System.Private.CoreLib/src/System/Threading/`). Em Windows existe também uma opção que
delega ao thread pool nativo do SO: `ThreadPool.Windows.cs` define o feature switch
`System.Threading.ThreadPool.UseWindowsThreadPool`.

Componentes confirmados no código:

1. **Fila de trabalho** (`ThreadPoolWorkQueue.cs`): estrutura em camadas (fila local por thread,
   filas globais, fila de alta prioridade). Detalhes de stealing no documento 12.
2. **Threads trabalhadoras** (`PortableThreadPool.WorkerThread.cs`): criadas como
   `new Thread(...)` com `IsThreadPoolThread = true` e `IsBackground = true`. O comentário no código diz
   que elas "devem começar com o ExecutionContext default sem transferir contexto", por isso usam
   `UnsafeStart`.
3. **Semáforo LIFO** (`LowLevelLifoSemaphore`): workers ociosos esperam nele. A escolha LIFO faz o
   worker mais recentemente ativo (cache quente) acordar primeiro.
4. **Contadores empacotados** (`PortableThreadPool.ThreadCounts.cs`): um único `ulong _data` guarda
   três campos de 16 bits: `NumProcessingWork` (shift 0), `NumExistingThreads` (shift 16) e
   `NumThreadsGoal` (shift 32). Atualizados por `InterlockedCompareExchange` sobre os 64 bits inteiros —
   três contadores mudam atomicamente sem lock. Há também um bit de "saturado" (`IsSaturated`, bit de sinal).
5. **Gate thread** (`PortableThreadPool.GateThread.cs`): thread auxiliar que acorda a cada
   `GateActivitiesPeriodMs = 500` ms para detectar starvation e fazer ajustes (inclusive cooperative blocking).
6. **Hill climbing** (`PortableThreadPool.HillClimbing.cs`): algoritmo que ajusta `NumThreadsGoal`
   buscando maximizar vazão (conclusões por segundo). Estados: `Warmup, Initializing, RandomMove,
   ClimbingMove, ChangePoint, Stabilizing, Starvation, ThreadTimedOut, CooperativeBlocking`.
   Intervalos padrão de amostragem: `DefaultSampleIntervalMsLow = 10`, `DefaultSampleIntervalMsHigh = 200`.
   Pode ser desligado (`System.Threading.ThreadPool.HillClimbing.Disable`).
7. **Blocking cooperativo** (`PortableThreadPool.Blocking.cs`): quando uma thread do pool vai bloquear
   (ex.: `Task.Wait` síncrono), o pool pode ser notificado e compensar com mais threads. Parâmetros:
   `ThreadsToAddWithoutDelay`, `ThreadsPerDelayStep`, `DelayStepMs`, `MaxDelayMs` — após as
   primeiras threads extras, cada nova thread é criada com atraso crescente (limitado por `MaxDelayMs`).
   Só vale para threads do pool (`Thread.CurrentThread.IsThreadPoolThread`) e quando
   `IsCooperativeBlockingEnabled`.

### Limites

- Mínimo padrão de threads: `Environment.ProcessorCount` (`PortableThreadPool.cs`: `_minThreads`).
- Máximo padrão: `short.MaxValue` como teto técnico (`MaxPossibleThreadCount`); `DefaultMaxWorkerThreadCount`
  varia por plataforma no `#if` do arquivo (aparecem os valores 10 para `TARGET_BROWSER`, `MaxPossibleThreadCount`
  e 1023 em outros ramos; consulte as linhas 18-25 do arquivo para o mapeamento exato de cada alvo).
- Configuráveis via `AppContext`/variáveis de ambiente (`System.Threading.ThreadPool.MinThreads`,
  `MaxThreads`, `ThreadsToKeepAlive`, `DisableStarvationDetection`, etc.).
- Timeout de thread ociosa padrão: `DefaultThreadPoolThreadTimeoutMs = 20 * 1000` (20 s) no ramo
  confirmado em `PortableThreadPool.cs` linha ~55.

### Ciclo de vida de uma requisição de trabalho

1. `ThreadPool.QueueUserWorkItem`/`UnsafeQueueUserWorkItem` (`ThreadPoolWorkQueue.cs`, classe parcial
   `ThreadPool`) enfileira via `s_workQueue.Enqueue(callback, forceGlobal)`.
2. `Enqueue` escolhe fila local (se chamado de thread do pool e `forceGlobal == false`, via
   `LocalPush`) ou global, e chama `ThreadPool.EnsureWorkerRequested()`.
3. A requisição usa um flag `_hasOutstandingThreadRequest` (em `PortableThreadPool.cs`): só a primeira
   requisição pendente (`Interlocked.Exchange(... , 1)`) realmente acorda/cria um worker — evita
   tempestade de wakeups.
4. O worker executa o loop de despacho (`ThreadPoolWorkQueue.Dispatch`): começa com `ExecutionContext` e
   `SynchronizationContext` limpos, desenfileira itens e os executa até esgotar o "quantum"
   (`DispatchQuantumMs = 30`) ou a fila. Após cada item reseta o estado de thread.
5. Se o worker perde um roubo (`missedSteal`), em vez de insistir ele pede outra thread, esperando que o
   dono da fila contendida consuma seus próprios itens (comentário no código).
6. Entre itens, o worker pergunta ao pool se deve parar (ajuste do hill climbing) e, ao estacionar,
   transfere itens da fila local para a global, para não atrasá-los.

### Detecção de starvation

No gate thread: se há requisição pendente (`_hasOutstandingThreadRequest != 0`) e passou tempo suficiente
desde o último dequeue (`SufficientDelaySinceLastDequeue`, limiar `DequeueDelayThresholdMs = 2 * 500 ms`),
e `NumProcessingWork < _maxThreads` e `>= NumThreadsGoal`, o goal sobe para `NumProcessingWork + 1`, hill
climbing é forçado ao estado `Starvation` e um worker é adicionado.

### Prioridades e outras filas

- `EnqueueAtHighPriority` e `highPriorityWorkItems`: fila prioritária consumida quando a thread entra em
  modo "processing high priority".
- Filas globais "atribuíveis" por grupo de processadores: `ProcessorsPerAssignableWorkItemQueue = 16`;
  só são usadas quando `Environment.ProcessorCount > 32`.
- Experimento de priorização (somente CoreCLR): `System.Threading.ThreadPool.PrioritizationExperiment`,
  default `false`, descrito como mitigação para inversão de prioridade em sync-over-async.
- Timers, IO completion e wait registrations têm arquivos próprios (`TimerQueue.*.cs`,
  `PortableThreadPool.IO.Windows.cs`, `PortableThreadPool.WaitThread.cs`).

## Por quê

- **Threads do SO são caras** (pilha, scheduling) e não escalam a milhares; reaproveitar workers e limitar
  o paralelismo ao número de CPUs reduz trocas de contexto. Isso é a razão do mínimo = `ProcessorCount`.
- **Throughput versus latência**: hill climbing procura o número de threads que maximiza vazão sem
  intervenção do usuário; o gate thread e o blocking cooperativo cobrem os casos em que o algoritmo de
  vazão não ajuda (tarefas que bloqueiam).
- **Escala em muitos núcleos**: contadores empacotados em um `ulong`, padding de cache-line em
  `_separated` (comentário: "ocupa seu próprio offset de cache-line") e filas por thread reduzem
  contenção e false sharing.
- **Threads virtualizadas**: o BOTR justifica a abstração de thread gerenciada pela necessidade do GC de
  achar referências e pela portabilidade (TLS, IDs próprios).
- **Multi-plataforma**: a mesma API sobre implementações diferentes (portable, Windows pool, browser
  com `DefaultMaxWorkerThreadCount` pequeno, WASI sem threads).

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Injeção lenta de threads além do mínimo | Evita explosão de threads | Latência quando o código bloqueia (sync-over-async) |
| Hill climbing | Auto-ajuste sem configuração | Comportamento difícil de prever; ruído de medição; código complexo |
| Fila local por thread | Menos contenção, boa localidade | Itens ficam "presos" em fila de thread ocupada; exige stealing |
| Worker LIFO | Cache quente | Threads antigas podem expirar (timeout) |
| Pool global único | Simples para o usuário | Uma carga mal-comportada afeta todas as outras (não há isolamento) |
| Bloqueio cooperativo | Compensa bloqueios | Só funciona onde o runtime sabe que bloqueou; atraso crescente |
| Configuração via AppContext/env | Operável sem recompilar | Muitos botões; comportamento difere por plataforma |

Observação: o próprio código mantém um experimento de priorização por causa de problemas de
sync-over-async, o que indica que o modelo "um pool, FIFO/LIFO misto" tem limitações conhecidas.

## Como aplicar na sua linguagem

Recomendações de design (inferências a partir do que o .NET faz; não são afirmações do código):

1. **Separe "unidade de trabalho" de "thread".** Ofereça uma primitiva (`spawn`, `task`) que enfileira
   trabalho num executor, e deixe a thread bruta como API de baixo nível.
2. **Um executor padrão com N = nº de CPUs** workers mais política de crescimento gradual para trabalhos
   bloqueantes. Se sua linguagem tiver bloqueio explícito (`blocking { ... }`), permita ao runtime
   compensar, como o blocking cooperativo do .NET.
3. **Estado de agendamento num único word atômico** (contagem em execução, existentes, meta). Facilita
   CAS sem locks.
4. **Evite wakeups redundantes** com um flag "há requisição pendente" (padrão `_hasOutstandingThreadRequest`).
5. **Fila local por worker + fila global + stealing** (veja doc 12), e permita ao usuário escolher entre
   "preferir local" (cache, throughput) e "justo" (FIFO global) como `preferLocal` do .NET.
6. **Quantum de despacho** (30 ms no .NET) para que periodicamente o worker devolva o controle ao runtime
   (timers, métricas, decisões de redução de threads).
7. **Threads dedicadas para trabalho longo** (`LongRunning` cria `new Thread` em vez de usar o pool) —
   evite que tarefas de duração indefinida esgotem o pool.
8. **Isolamento**: considere múltiplos executores (por exemplo, um para I/O, um para CPU) se quiser evitar
   o problema do pool único. O .NET não oferece isso por padrão.
9. **Se houver GC**, desenhe o modelo de threads em torno de safe points e do estado "preemptive"/"cooperative"
   como no BOTR; se não houver, essa complexidade some.
10. **Alvos single-thread**: separe por feature flag em tempo de build, como as variantes
    `Thread.SingleThreading.cs`.

## Referências

- `docs/design/coreclr/botr/threading.md`
- `src/libraries/System.Private.CoreLib/src/System/Threading/PortableThreadPool.cs`
- `.../PortableThreadPool.WorkerThread.cs`, `.../PortableThreadPool.GateThread.cs`
- `.../PortableThreadPool.ThreadCounts.cs`, `.../PortableThreadPool.HillClimbing.cs`
- `.../PortableThreadPool.Blocking.cs`
- `.../ThreadPoolWorkQueue.cs`, `.../ThreadPool.Windows.cs`, `.../ThreadPool.Unix.cs`
- `.../Thread.cs`, `.../Thread.Multithreading.cs`, `.../Thread.SingleThreading.cs`
- `.../Tasks/ThreadPoolTaskScheduler.cs`
- `.../LowLevelLifoSemaphore.cs`
