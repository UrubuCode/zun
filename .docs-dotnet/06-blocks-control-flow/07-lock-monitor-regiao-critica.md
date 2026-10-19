# lock e Monitor (entrada/saída de região crítica)

`lock` é, como `using`, açúcar de compilador sobre um par `Enter`/`Exit` protegido por `try/finally`. O interessante para quem projeta uma linguagem é (1) o **protocolo `lockTaken`** que torna o par seguro contra exceções assíncronas entre o `Enter` e o `try`, (2) o fato de que o JIT usa **exatamente o mesmo padrão** para métodos `synchronized`, e (3) a evolução do .NET de "todo objeto é um monitor" para um tipo dedicado, `System.Threading.Lock`.

## Mecanismo no .NET

### 1. A API do runtime: `Monitor`

`src/libraries/System.Private.CoreLib/src/System/Threading/Monitor.cs`:

```csharp
public static void Enter(object obj)                       // NoInlining -> ObjectHeader.AcquireThinLock(obj)
public static void Enter(object obj, ref bool lockTaken)   // lança ArgumentException se lockTaken já é true
public static bool TryEnter(object obj) / TryEnter(obj, int ms) / TryEnter(obj, ref bool lockTaken)
public static void Exit(object obj)                         // ArgumentNullException.ThrowIfNull + ObjectHeader.Release
public static bool IsEntered(object obj)
```

Detalhes verificáveis no código:

- `Enter(object, ref bool lockTaken)` primeiro verifica `if (lockTaken) ThrowLockTakenException();`, ou seja, exige que o chamador inicialize `lockTaken = false`; só depois de adquirir faz `lockTaken = true`. Comentário: "Aggressively inline lockTaken check as it is likely to be optimized away".
- `Exit(obj)` valida `obj` com `ArgumentNullException.ThrowIfNull` e chama `ObjectHeader.Release(obj)`. A existência de `SynchronizationLockException.cs` na mesma pasta e o `exception cref` em `Lock.Scope.Dispose` indicam que sair sem ser dono lança essa exceção.
- Existem helpers privados `SynchronizedMethodEnter/Exit(object obj, ref bool lockTaken)` usados para métodos `synchronized`; `SynchronizedMethodExit` começa com `if (!lockTaken) return;`.

### 2. O padrão `lockTaken` (por que existe)

Convenção de linguagem (o compilador C# não está neste repo): `lock (x) { body }` é expandido para

```csharp
bool lockTaken = false;
try {
    Monitor.Enter(x, ref lockTaken);
    body
}
finally {
    if (lockTaken) Monitor.Exit(x);
}
```

O `Enter` ocorre **dentro** do `try`, e a flag diz se a aquisição aconteceu. Isso fecha a janela em que uma exceção chegue entre `Enter` e o início do `try`. A evidência mais forte neste repositório é o comentário do JIT em `src/coreclr/jit/flowgraph.cpp` (`fgAddSyncMethodEnterExit`, linha ~1482), que gera para métodos marcados `synchronized` (flag `CORINFO_FLG_SYNCH`):

```
unsigned byte acquired = 0;
try {
    Monitor.Enter(<lock object>, &acquired);
    *** código do usuário ***
    Monitor.ExitIfTaken(<lock object>, &acquired);
} fault {
    Monitor.ExitIfTaken(<lock object>, &acquired);
}
```

O comentário explica o raciocínio: "If an exception occurs before the lock is acquired, but within the 'try' (extremely unlikely, but possible), 'acquired' will be 0, and the monitor exit call will quickly return without attempting to release the lock." E justifica o uso de **um único bloco de retorno**, citando a ECMA-335: `ret` "Shall not be enclosed in any protected block"; saída de um `try` só por `leave`; "Control is not permitted to simply fall through the end of a method". Note que o esboço do comentário usa `fault` (não `finally`) e chama o exit explicitamente no caminho normal.

Observe também em `importer.cpp` (~linha 11991) o temporário `lvaMonAcquired` ("Synchronized method monitor acquired boolean"), e os helpers `CORINFO_HELP_MON_ENTER`/`CORINFO_HELP_MON_EXIT` em `src/coreclr/inc/corinfo.h` (linhas 415-416).

### 3. Onde o estado do lock mora: o cabeçalho do objeto

Em .NET, qualquer objeto de referência pode ser usado em `lock`. O estado vive no **ObjHeader**, uma palavra antes do ponteiro do objeto. Comentário de `src/coreclr/vm/syncblk.h` (linha ~22, "#SyncBlockOverview"):

> Every Object is preceded by an ObjHeader (at a negative offset). The header can store one of a thin lock, a hash code, or a nonzero index into the process-global g_pSyncTable when the object needs a SyncBlock to store more state than can fit into ObjHeader.

Layout do **thin lock** (`syncblk.h`, linhas ~62-100):

| Bits | Uso |
|---|---|
| 0-15 (`SBLK_MASK_LOCK_THREADID`) | id do thread dono (0 = livre) |
| 16-21 (`SBLK_MASK_LOCK_RECLEVEL`) | nível de recursão (64 níveis) |
| 27 (`BIT_SBLK_IS_HASH_OR_SYNCBLKINDEX`) | se 1, o resto é hash code ou índice de SyncBlock |
| 26 (`BIT_SBLK_IS_HASHCODE`) | distingue hash de índice (bits 0-25) |

Se o thread id não cabe, a recursão estoura ou há contenção, o lock é **inflado** para um `SyncBlock`/objeto `Lock` completo. O código gerenciado do CoreCLR para isso está em `src/coreclr/System.Private.CoreLib/src/System/Threading/ObjectHeader.CoreCLR.cs`: `AcquireThinLock` e `TryAcquireThinLock` (fast path inline "free lock, fat lock"; os casos raros vão para `TryAcquireUncommon`). Comentários do arquivo descrevem a política: se um thin lock está mantido por outro thread por mais de ~5 microssegundos, ele é inflado e o spinning adaptativo do `Lock` "fat" decide; adota-se back-off **linear** (0, 1, 2, ...) por favorecer micro-contenção. `GetLockObject`/`GetOrCreateLockObject` fazem o mapeamento objeto para `Lock` (`Monitor.cs`, região "Object->Lock mapping").

### 4. `System.Threading.Lock`: um tipo dedicado

`src/libraries/System.Private.CoreLib/src/System/Threading/Lock.cs` define `public sealed class Lock` ("Provides a way to get mutual exclusion in regions of code between different threads"). Campos: `_owningThreadId`, `_state` (layout em `State`), `_recursionCount`, `_spinCount` (spinning adaptativo), `_waitEventOrCondition`. Constantes de spin: `DefaultMaxSpinCount = 22`, `DefaultAdaptiveSpinPeriod = 100`.

API: `Enter()`, `TryEnter()`, `TryEnter(int)`, `TryEnter(TimeSpan)`, `Exit()`, `IsHeldByCurrentThread`, e:

```csharp
public Scope EnterScope() => new Scope(this, EnterAndGetCurrentThreadId());   // linha ~155
public ref struct Scope { ... public void Dispose() { ...lockObj.Exit(_currentThreadId); } }
```

`Scope` é um `ref struct` com `Dispose()`; guarda o `_currentThreadId` para que `Exit` seja barato e `Dispose` seja idempotente (zera `_lockObj`). A idéia: `using (lock.EnterScope()) { ... }`, ou seja, o `lock` do C# sobre um `Lock` se reduz ao padrão `using` do doc 4 e dispensa a flag `lockTaken` (o `EnterScope` retorna o escopo só após aquisição bem-sucedida; se lançar, não há escopo a descartar). Esta tradução específica do compilador para o tipo `Lock` é convenção de linguagem, não verificada aqui.

Notas de projeto presentes nos comentários de `Lock.cs`:
- "Lock must not have a static (class) constructor, as Lock itself is used to synchronize class construction. If Lock has its own class constructor, this can lead to infinite recursion. All static data in Lock must be lazy-initialized."
- O limite de recursão é "implementation-defined" e gera `LockRecursionException` ao ser atingido; `Exit` por thread não dono lança `SynchronizationLockException`.

### 5. Semântica de exceções assíncronas e `finally`

O `finally` do `lock` roda na 2ª passada (doc 3). Em `ThreadAbort` (hoje não suportado no .NET Core, ver `clr-abi.md` seção "ThreadAbortException considerations"), o JIT mantém código compatível: o endereço de retorno do "call finally" deve ficar fora do corpo do try e dentro das regiões externas, e "cloned finally" impede que um abort dispare antes de entrar no handler.
### 6. `Wait`/`Pulse` e a espera em monitor

`Monitor.Wait/Pulse/PulseAll` (também em `Monitor.cs`) operam sobre o `Lock` associado ao objeto via `GetLockObject`. Há `UnsupportedOSPlatform("browser")` em `Wait` quando não há threads gerenciadas em WASM (`#if !FEATURE_WASM_MANAGED_THREADS`). O parâmetro `exitContext` é ignorado ("Remoting is not supported").

## Por quê

1. **Segurança a exceções.** O padrão `lockTaken` + `finally` garante que o lock é liberado em todos os caminhos de saída da região, e evita liberar um lock que não foi adquirido.
2. **Reuso de regiões protegidas.** O mesmo mecanismo de IL (try/finally ou try/fault) serve a `lock`, `using`, métodos `synchronized`.
3. **Custo mínimo no caso sem contenção.** Thin lock: algumas instruções CAS sobre o cabeçalho do objeto, sem alocar nada, para o caso comum (comentário: "taking and releasing the lock takes 2 CAS instructions + some overhead").
4. **Compatibilidade histórica.** "Qualquer objeto pode ser lock" foi uma decisão antiga; o tipo `Lock` dedicado corrige suas desvantagens (lock acidental em `this`/strings, cabeçalho de objeto sobrecarregado) mantendo a interoperabilidade via `Monitor`.
5. **Escopo como valor.** `Lock.Scope` é `ref struct`, sem alocação e sem poder escapar do stack (não pode ser capturado por lambda ou usado em `async`, o que evita liberar o lock em outro thread).

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Lock em qualquer objeto (cabeçalho) | Sem declarar campo; amplamente usável | Cabeçalho disputado com hash code e estado de GC; bugs de lock em objeto público/`this`/string |
| Thin lock com inflação | Rápido no caso sem contenção | Complexidade (bits, id de thread de 16 bits, níveis de recursão limitados a 64) |
| `lockTaken` por referência | Segurança com exceções entre Enter e try | API estranha; programadores esquecem de inicializar `false` |
| `Lock` dedicado + `Scope` | Estado próprio, API clara, sem boxing | Dois mundos (`Monitor` e `Lock`) coexistem |
| Spin adaptativo | Menos context switches em seções curtas | Consumo de CPU sob contenção; heurísticas calibradas por hardware |
| Recursão suportada | Código reentrante simples | Esconde bugs de design; custo de contagem |
| Lock não atravessa `await` | Evita deadlocks por troca de thread | Para locks assíncronos é preciso outro primitivo (`SemaphoreSlim`) |
| Exceções não liberam em crash | Falha rápida | Em exceção não tratada nenhum `finally` roda (doc 3) |

## Como aplicar na sua linguagem

1. **Defina `lock` como açúcar sobre um tipo/trait de lock explícito**, não sobre qualquer objeto. Evita a dependência do cabeçalho do objeto e os bugs conhecidos de "lock em qualquer coisa".
2. **Escolha o protocolo de segurança**:
   - estilo `lockTaken` (aquisição dentro do `try`, flag de aquisição), ou
   - estilo guard/escopo (`EnterScope()` retornando um valor com `Dispose`), que é estruturalmente mais simples e não precisa de flag.
3. **Torne o guard um tipo "não escapável"** (como `ref struct`): proíba capturar o guard em closures e em state machines assíncronas. Isso impede que o lock seja liberado de outra thread.
4. **Use o mesmo mecanismo de região protegida** do seu `using`/`defer` (docs 2 e 4). Se sua IR distingue `fault` de `finally`, o `fault` serve quando a saída normal já faz o `Exit` explicitamente (como no `synchronized` do JIT).
5. **Defina erros claros**: `Exit` por não dono, estouro de recursão, lock já adquirido (`lockTaken` verdadeiro na entrada lança `ArgumentException` no .NET).
6. **Se implementar lock no cabeçalho**: reserve bits para thread id, nível de recursão e uma indicação de "inflado"; especifique a política de inflação (tempo de espera, contenção) e de spinning, e meça. Se não, comece com um mutex simples; otimize depois.
7. **Cuide da inicialização**: o `Lock` do .NET não pode ter construtor estático porque é usado para sincronizar construção de classes. Se sua linguagem tem inicialização preguiçosa de estáticos com locks, evite dependência circular.
8. **Decida a interação com `async`**: proíba (erro de compilação) ou permita `await` dentro de região crítica? O .NET proíbe em `lock` (C#), e fornece primitivos assíncronos separados.
9. **Documente a semântica de memória** (acquire/release): veja `docs/design/specs/Memory-model.md`.

## Apêndice: ciclo de vida do lock de um objeto

Descrição fiel aos comentários de `syncblk.h` e `ObjectHeader.CoreCLR.cs`:

1. **Livre**: o cabeçalho tem thread id 0 nos bits 0-15 e `BIT_SBLK_IS_HASH_OR_SYNCBLKINDEX` limpo.
2. **Thin lock**: `Monitor.Enter` faz CAS no cabeçalho para colocar o id do thread; recursão incrementa o campo de nível (bits 16-21, até 64 níveis).
3. **Contenção curta**: o thread que não consegue gira (spin) com back-off linear; os comentários estimam ~5 microssegundos como limite antes de inflar e discutem que, com 2 CAS + overhead, "it is unlikely to see more than 8-10 threads contending for the lock without inflating it".
4. **Inflação**: o lock vira um `Lock` completo (campos de `Lock.cs`: dono, estado, contagem de recursão, spin adaptativo, evento de espera); o cabeçalho passa a guardar um índice (`BIT_SBLK_IS_HASH_OR_SYNCBLKINDEX` ligado) para a tabela de SyncBlocks.
5. **Hash code**: o mesmo cabeçalho pode guardar o hash (`BIT_SBLK_IS_HASHCODE`); se o objeto precisa de hash **e** lock, é necessário um SyncBlock. Esse acoplamento é um custo da decisão "qualquer objeto é lock".
6. **Liberação**: `Monitor.Exit` chama `ObjectHeader.Release`; a lógica detalhada (decremento de recursão, liberação do dono) está em `ObjectHeader.CoreCLR.cs` e `Lock.cs` e não foi reproduzida aqui.
7. **GC**: entradas da tabela de SyncBlocks guardam referência fraca ao objeto; SyncBlocks de objetos mortos são devolvidos ("During GC, entries for dead objects are returned to the free list").

## Apêndice: perguntas de design

1. **Qualquer objeto como lock ou tipo dedicado?** O .NET tem os dois (`Monitor` e `Lock`); um tipo dedicado evita acoplamento com hash e cabeçalho.
2. **Recursivo ou não?** O `Lock` do .NET é recursivo; muitos sistemas modernos preferem não-recursivo.
3. **Justo ou não justo?** O `Lock` mantém `MaxDurationMsForPreemptingWaiters = 100` (constante em `Lock.cs`), sinal de política híbrida de preempção de waiters; a política exata não é detalhada aqui.
4. **Protocolo de segurança com exceções**: `lockTaken` vs. guard.
5. **Fila de espera e Wait/Pulse (condition variables)**: `Monitor.Wait/Pulse` usam o `Lock` do objeto; `Lock.Condition` é criado sob demanda (`GetOrCreateCondition`).
6. **Detecção de deadlock?** Não há; `TryEnter` com timeout é a ferramenta.
7. **Interação com async**: o lock é por thread; `await` dentro de `lock` seria perigoso (linguagem proíbe).
8. **Inicialização**: o `Lock` não pode ter construtor estático (usado na sincronização de construção de classes).
9. **Modelo de memória**: ver `docs/design/specs/Memory-model.md`.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/Monitor.cs` (`Enter`, `TryEnter`, `Exit`, `SynchronizedMethodEnter/Exit`, mapeamento objeto para `Lock`).
- `src/libraries/System.Private.CoreLib/src/System/Threading/Lock.cs` (`Lock`, `EnterScope`, `Scope`).
- `src/coreclr/System.Private.CoreLib/src/System/Threading/ObjectHeader.CoreCLR.cs` (thin locks, `AcquireThinLock`, `TryAcquireThinLock`, `GetLockObject`).
- `src/coreclr/vm/syncblk.h`, `syncblk.cpp` (#SyncBlockOverview, máscaras `SBLK_*`, `ObjHeader::GetSyncBlock`).
- `src/coreclr/jit/flowgraph.cpp` (`fgAddSyncMethodEnterExit`, linhas ~1482-1720), `src/coreclr/jit/importer.cpp` (`lvaMonAcquired`).
- `src/coreclr/inc/corinfo.h` (`CORINFO_HELP_MON_ENTER`, `CORINFO_HELP_MON_EXIT`, flag `CORINFO_FLG_SYNCH`).
- `docs/design/coreclr/botr/clr-abi.md` (ThreadAbort, invocação de finally), `docs/design/coreclr/botr/threading.md`, `docs/design/specs/Memory-model.md`.

> Nota de confiança: a expansão exata do `lock` pelo compilador C# (inclusive a variante para o tipo `Lock`) é convenção do compilador; o padrão equivalente usado pelo JIT para métodos `synchronized` é confirmado em `flowgraph.cpp`.
