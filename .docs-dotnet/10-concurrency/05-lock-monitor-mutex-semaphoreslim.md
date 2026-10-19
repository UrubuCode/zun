# lock, Monitor, Mutex e SemaphoreSlim

Primitivas de exclusão mútua e sinalização do .NET: `System.Threading.Lock` (tipo dedicado novo),
`Monitor` (lock sobre qualquer objeto), `Mutex` (objeto do SO, inclusive nomeado entre processos),
`SemaphoreSlim` (contagem + espera assíncrona), além de `SpinWait`/`SpinLock`.

## Mecanismo no .NET

### `Lock` (`System.Threading/Lock.cs`)

Classe selada, "exclusão mútua entre threads; recursiva". Campos confirmados:

```
private int  _owningThreadId;   // dono atual
private uint _state;            // bits de estado (layout abaixo)
private uint _recursionCount;
private short _spinCount;       // spin adaptativo
private ushort _waiterStartTimeMs;
```

Layout de `_state` (struct `State`, ~linha 992):

| Bits | Significado |
|---|---|
| 0 | `IsLocked` |
| 1 | `ShouldNotPreemptWaiters` |
| 2-4 | contagem de spinners (até 7) |
| 5 | `IsWaiterSignaledToWake` |
| 6 | `UseTrivialWaits` |
| 7-31 | contagem de waiters |

Caminho rápido (`TryEnter_Inlined`): pega o `ManagedThreadId` atual e tenta `State.TryLock(this)` (um CAS no
`_state`); se ok, grava `_owningThreadId`. Caso contrário vai ao `TryEnterSlow` (NoInlining):

1. Se já é o dono → incrementa `_recursionCount` (lança `LockRecursionException` em overflow de contador).
2. `timeoutMs == 0` → retorna falha (TryEnter).
3. `Debugger.NotifyOfCrossThreadDependency()` — avisa o depurador que a thread vai esperar por outra.
4. **Spin** antes de dormir: constantes `DefaultMaxSpinCount = 22`, `DefaultAdaptiveSpinPeriod = 100`,
   `SpinSleep0Threshold = 10`. O primeiro spinner faz um spin completo para medir se spinning funciona; o
   resultado ajusta `_spinCount` (positivo = spin típico, zero = tentar, negativo = contador de contenções
   até a próxima tentativa). Em máquina de um só processador, `s_isSingleProcessor` evita girar.
5. **Wait** num `AutoResetEvent` criado sob demanda (`WaitEvent`), contando waiters no `_state`.
6. Preempção de waiters: `MaxDurationMsForPreemptingWaiters = 100` — enquanto o waiter mais antigo esperou
   menos que isso, threads novas podem "furar a fila" (barging, melhor throughput); depois o bit
   `ShouldNotPreemptWaiters` impede, para evitar starvation.

`EnterScope()` retorna `Lock.Scope` (**`ref struct`** descartável): `using (lockObj.EnterScope()) { ... }`.
O compilador C# mais novo usa isso para `lock (x)` quando `x` é do tipo `Lock`. O comentário do arquivo
alerta: **`Lock` não pode ter construtor estático**, pois o próprio `Lock` sincroniza construção de classes;
dados estáticos são inicializados lazy (`s_staticsInitializationStage`).

Também há `Condition` (`Condition.cs`) ligada ao `Lock` (visível pelos campos `_waitEvent`).

### `Monitor` (`Monitor.cs`, `Monitor.CoreCLR`)

API clássica `Enter/Exit/TryEnter/Wait/Pulse/PulseAll/IsEntered` sobre **qualquer objeto**, também usada por
`lock (obj)` quando `obj` não é `Lock`. No Native AOT/portable (`Monitor.cs` em CoreLib compartilhado), `Enter`
chama `ObjectHeader.AcquireThinLock(obj)` e `GetLockObject(obj)` mapeia o objeto a um `Lock`, onde `Wait`/`Pulse`
são implementados: "Object->Lock mapping".

Em `src/coreclr/System.Private.CoreLib/.../ObjectHeader.CoreCLR.cs`, o cabeçalho de 4 bytes antes do
ponteiro de MethodTable guarda (quando bit 27 está limpo) um **thin lock**: 16 bits de thread id (valor 0 = livre;
65535 ids), 6 bits de nível de recursão (64 níveis). Quando há contenção, wait/pulse ou hash code concorrente,
o header aponta para um **índice de sync block** (mesmo layout do CLR clássico: bit 26 = é hash code;
bit 27 = hash-ou-syncblock-index; bits 31/30/29 reservados ao GC; bit 28 = spinlock de modificação do índice).
Isto é, o custo por objeto do lock "gratuito" é zero bytes adicionais no caso comum, e um sync block só é
alocado sob demanda. O código nativo correspondente está em `src/coreclr/vm/syncblk.cpp`/`syncblk.h`.

Métodos marcados `[MethodImpl(MethodImplOptions.Synchronized)]` usam `SynchronizedMethodEnter/Exit`.
`Monitor.LockContentionCount` expõe `Lock.ContentionCount`.

Semântica de memória (doc 04): `Monitor.Enter` = acquire, `Monitor.Exit` = release.

### `Mutex` (`Mutex.cs`, `Mutex.Windows.cs`, `Mutex.Unix.cs`, `NamedMutex.Unix.cs`)

`Mutex : WaitHandle` — objeto do SO (Windows) ou emulação (Unix, com `WaitSubsystem.*.Unix.cs` e
`NamedMutex.Unix.cs`), com **nomes** para sincronização **entre processos**. Os novos construtores recebem
`NamedWaitHandleOptions` (`CurrentUserOnly`, `CurrentSessionOnly`, padrão `true`/`true` segundo os comentários).
Tem afinidade de thread (quem libera deve ser o dono) e `AbandonedMutexException` quando o dono morre sem liberar.
É muito mais caro que `Lock`/`Monitor` (transição para o kernel / subsistema de espera).

### `SemaphoreSlim` (`SemaphoreSlim.cs`)

- Estado: `volatile int m_currentCount`, `m_waitCount`, e um objeto de lock interno
  (`m_lockObjAndDisposed`, `StrongBox<bool>`) usado com `Monitor.Enter/Wait/Pulse`.
- `Wait` síncrono: se `m_currentCount == 0`, gira (`SpinWait`, `SpinCountForSpinBeforeWait * 4`) antes de
  entrar no `Monitor` — o comentário diz que `Monitor.Enter` + `Monitor.Wait` é bem mais caro que esperar um
  evento, e que o spin antes mostrou ganho medido.
- `WaitAsync(...)` retorna `Task`/`Task<bool>`: mantém uma **lista duplamente ligada de `TaskNode`**
  (`m_asyncHead/m_asyncTail`), onde `TaskNode : Task<bool>` criado com `TaskCreationOptions.RunContinuationsAsynchronously`.
  `Release` completa nós em ordem. Se há waiters assíncronos, waiters síncronos também enfileiram como assíncronos
  para preservar ordem (ramo `m_asyncHead is not null`).
- Cancelamento e timeout integrados (`CancellationToken`, `millisecondsTimeout`).
- Por usar `RunContinuationsAsynchronously`, completar um `Release` nunca executa continuação do waiter dentro
  do lock — evita reentrância/deadlock.
- `AvailableWaitHandle` cria `ManualResetEvent` sob demanda.

### Primitivas de espera de baixo nível

`SpinWait.cs`: `YieldThreshold = 10`, `Sleep0EveryHowManyYields = 5`, `DefaultSleep1Threshold = 20`,
`SpinCountForSpinBeforeWait = 35` (1 em single-processor). Escalona de `Thread.SpinWait` → `Yield`/`Sleep(0)` →
`Sleep(1)`. `SpinLock.cs` (usado por `WorkStealingQueue.m_foreignLock`), `ManualResetEventSlim`,
`ReaderWriterLockSlim`, `LowLevelLock`/`LowLevelMonitor`/`LowLevelFutex.*` (locks internos do runtime, futex
em Linux), `WaitHandle` e `WaitSubsystem` (Unix) que implementam `WaitAny/WaitAll` portáveis.

## Por quê

- **Lock barato no caso sem contenção**: um CAS no estado e uma escrita do dono; thin lock no header evita
  alocar qualquer coisa.
- **Spin adaptativo**: seções críticas curtas são vencidas girando, evitando o custo de bloquear/acordar
  (troca de contexto), mas sem desperdiçar CPU quando spinning não ajuda (contador de eficácia).
- **Barging controlado** (preempção de waiters até 100 ms): maximiza throughput sem causar inanição.
- **`Lock` separado de `Monitor`**: `Monitor` precisa suportar qualquer objeto + wait/pulse + sync blocks, o
  que o torna pesado. Um tipo `Lock` dedicado simplifica a implementação e permite `Scope` (RAII por `using`).
- **`SemaphoreSlim` assíncrono**: `await` não pode manter lock de thread; semáforo assíncrono permite limitar
  concorrência sem bloquear threads.
- **Mutex nomeado**: coordenação inter-processo, caso que primitivas em processo não cobrem.

## Trade-offs

- Locks recursivos permitem reentrada acidental e escondem design ruim (mas evitam auto-deadlock).
- `lock` bloqueia thread do pool → starvation sob carga (doc 01); não se pode `await` dentro de `lock`.
- Spin gasta CPU; mal ajustado, prejudica em máquinas com muitas threads/containers limitados.
- Barging reduz justiça (fairness) e piora a latência de cauda de waiters individuais.
- Thin locks limitam: 65535 thread IDs e 64 níveis de recursão antes de inflar para sync block; inflar custa.
- Lock sobre objeto arbitrário (`lock(this)`, `lock(typeof(X))`) expõe o lock publicamente — fonte de deadlocks.
  Um tipo `Lock` dedicado evita isso.
- `Mutex` do SO: ordens de grandeza mais lento, mas a única opção para nomes entre processos.
- `SemaphoreSlim` não é recursivo e não tem "dono": liberar mais que o permitido lança `SemaphoreFullException`.

## Como aplicar na sua linguagem

1. **Ofereça um tipo `Lock` dedicado**, não "monitor em qualquer objeto" — evita overhead de header,
   exposição pública e permite RAII (`using`/`defer`/escopo). Se sua linguagem tem GC e objetos pequenos,
   ainda considere o thin-lock inline se o custo de 1 palavra por lock for aceitável.
2. **Implemente fast path em 1 CAS**, e slow path com spin adaptativo + parking (futex/evento).
3. **Evite criar o objeto de espera até haver contenção** (lazy, como `WaitEvent`).
4. **Decida a política de justiça**: barging com limite de tempo é uma boa base (valor do .NET: 100 ms).
5. **Semáforo assíncrono** como primitiva de primeira classe, com fila FIFO e continuações assíncronas
   (nunca executar código do usuário dentro da seção crítica interna).
6. **Condições**: se oferecer wait/notify, amarre à `Lock` (o .NET tem `Condition` ligado a `Lock`).
7. **Proíba (ou torne ineficiente de propósito) suspender com lock síncrono mantido** — em linguagens com
   async, faça o compilador avisar sobre `await` dentro de `lock` (o .NET proíbe `await` em `lock`).
8. **Locks entre processos** como tipo separado e explícito, com opções de escopo/segurança.
9. **Detecção de deadlock/diagnóstico**: o .NET expõe `LockContentionCount` e notifica o depurador antes de
   esperar; reserve ganchos semelhantes.
10. **Documente a semântica de memória** (acquire/release) de cada operação (doc 04).

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/Lock.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Monitor.cs`
- `src/coreclr/System.Private.CoreLib/src/System/Threading/ObjectHeader.CoreCLR.cs`
- `src/coreclr/vm/syncblk.cpp`, `src/coreclr/vm/syncblk.h`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Mutex.cs`, `Mutex.Windows.cs`, `Mutex.Unix.cs`, `NamedMutex.Unix.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/SemaphoreSlim.cs`
- `.../Threading/SpinWait.cs`, `SpinLock.cs`, `Condition.cs`, `ReaderWriterLockSlim.cs`, `ManualResetEventSlim.cs`
- `.../Threading/LowLevelLock.cs`, `LowLevelFutex.Unix.cs`, `WaitSubsystem.Unix.cs`
- `docs/design/specs/Memory-model.md` (semântica de lock)
