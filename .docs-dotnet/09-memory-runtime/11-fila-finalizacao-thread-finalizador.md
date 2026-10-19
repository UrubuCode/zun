# Fila de finalização e thread de finalizador

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.
> Código: `src/coreclr/gc/finalization.cpp` (`CFinalize`), `src/coreclr/gc/gcpriv.h` (classe
> `CFinalize`, linhas ~5688-5780), `src/coreclr/vm/finalizerthread.cpp`,
> `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs`.

## Mecanismo no .NET

### Finalização "premortem": o que é

Um objeto cuja classe tem finalizador (`~Classe()`, que o runtime chama de método `Finalize`)
precisa ter esse método executado **depois** que se torna inalcançável e **antes** de sua memória
ser reaproveitada. Em tempo de compilação o GC é construído com
`FEATURE_PREMORTEM_FINALIZATION` (`gcpriv.h:193`).

Fluxo, resumido:

1. **Alocação**: ao alocar um objeto cujo tipo tem finalizador, o EE passa a flag
   `GC_ALLOC_FINALIZE` (`gcinterface.h:1094`, "keep in sync with GC_ALLOC_FLAGS in
   GC.CoreCLR.cs"). O GC chama `GCHeap::RegisterForFinalization(gen, obj)`
   (`interface.cpp:2496`) que delega a `CFinalize::RegisterForFinalization`
   (`finalization.cpp:172`).
2. **Fila de registro (`CFinalize`)**: o objeto entra numa estrutura segmentada **por
   geração** (ver abaixo). O GC **não considera** esta fila como raiz forte durante o mark:
   isso é essencial para o objeto poder "morrer".
3. **Coleta**: depois do mark, `CFinalize::ScanForFinalization` percorre as gerações
   condenadas e, para cada objeto registrado que **não foi promovido** (isto é, não marcado =
   inalcançável), decide o que fazer; ver abaixo.
4. **Ressurreição temporária**: os objetos movidos para as filas "f-reachable" são **promovidos**
   (`GcScanRoots(pfn, ...)`) para que não sejam coletados nem seus objetos referenciados.
5. **Execução**: a thread de finalizador chama o finalizador de cada objeto; só na **próxima**
   coleta (que já não encontra o objeto na fila) a memória é reclamada.

### A estrutura `CFinalize`

`gcpriv.h` (comentário no código): uma única matriz `m_Array` dividida em **segmentos**
delimitados por `m_FillPointers`:

```
[ gen_N ... gen_1 gen_0 ... (uma por geração) | CriticalFinalizerListSeg | FinalizerListSeg | FreeListSeg ]
```

- `ExtraSegCount = 2` (os dois segmentos extras acima); `FinalizerListSeg =
  total_generation_count + 1`; `CriticalFinalizerListSeg = total_generation_count`;
  `FreeListSeg = total_generation_count + ExtraSegCount`.
- Um objeto registrado fica no segmento da sua **geração atual** (`gen_segment(gen)`). Isso
  permite que, numa coleta de gen*N*, o GC varra só os segmentos das gerações condenadas
  (`startSeg = gen_segment(gen)` até `gen_segment(0)` em `ScanForFinalization`).
- Mover um item entre segmentos (`MoveItem`) é feito trocando o item com o elemento de fronteira
  do segmento adjacente e ajustando o ponteiro de preenchimento: custo O(nº de segmentos), não
  O(tamanho da fila).
- Acesso protegido por spin lock `lock` (`EnterFinalizeLock`/`LeaveFinalizeLock`).
- A matriz cresce (`GrowArray`); o tamanho inicial é 100 entradas
  (`INITIAL_FINALIZER_ARRAY_SIZE`).
- Após a coleta, `RelocateFinalizationData` atualiza os ponteiros se os objetos foram movidos
  (arquivo 02). Em GC Server há uma fila **por heap**, com `MergeFinalizationData` /
  `SplitFinalizationData` quando o número de heaps muda (arquivo 04).

### O que `ScanForFinalization` decide (código lido)

Para cada objeto não promovido (`!IsPromoted`), em `finalization.cpp:~348`:

1. `GCToEEInterface::EagerFinalized(obj)` verdadeiro (o EE já tratou) -> move para `FreeListSeg`.
2. Se o header do objeto tem o bit `BIT_SBLK_FINALIZER_RUN` (o programa chamou
   `GC.SuppressFinalize`, ou o finalizador já rodou) -> remove da fila (`FreeListSeg`) e **zera**
   o bit, para que seja colocado de volta se for ressuscitado e re-registrado.
3. Caso contrário: `m_PromotedCount++` e move para
   `CriticalFinalizerListSeg` se `HasCriticalFinalizer()`, senão para `FinalizerListSeg`.

Se algo foi movido para as listas "f-reachable", o GC **promove essas raízes**
(`GcScanRoots`) e marca `settings.found_finalizers`. Em GC concorrente, também chama
`GCToEEInterface::EnableFinalization(true)` para acordar a thread de finalizador.

Tipo especial: **critical finalizers** (`CriticalFinalizerObject`, usados por
`SafeHandle`): ficam em segmento próprio; `GetNextFinalizableObject(only_non_critical)` devolve
primeiro os não críticos, e só depois os críticos (finalization.cpp:~245). **[confirmado]**

### Sinalização do GC para a thread (`schedule_finalizer_work`)

`gc_heap::schedule_finalizer_work(FinalizerWorkItem*)` (`finalization.cpp:13`) empilha itens de
trabalho numa lista encadeada **lock-free** (`Interlocked::CompareExchangePointer`) e, se a
lista estava vazia, chama `GCToEEInterface::EnableFinalization(true)`. Esses itens são lidos
pela thread de finalizador via `GetExtraWorkForFinalization()`.

### A thread de finalizador (`finalizerthread.cpp`)

- **Uma única thread dedicada**, criada por `FinalizerThread::FinalizerThreadCreate()`
  (linha ~771); nome `".NET Finalizer"` (linha ~799). É marcada como *background*
  (`SetBackground(TRUE)`).
- Eventos (`CLREvent`): `hEventFinalizer` (auto-reset, "há trabalho"),
  `hEventFinalizerDone` (manual, "terminei de drenar"), `hEventFinalizerToShutDown`.
  `EnableFinalization()` faz `hEventFinalizer->Set()`.
- **Espera** (`WaitForFinalizerEvent`): primeiro espera só o evento do finalizador por **2
  segundos**; se estoura o tempo, espera em conjunto o evento do finalizador e o
  `kLowMemoryNotification` (notificação de pouca memória do SO), nessa ordem de prioridade para
  que o low-memory não "mate de fome" o finalizador, já que o finalizador pode ajudar a
  corrigir a falta de memória. **[confirmado]** nos comentários.
- **Prioridade**: na primeira iteração tenta `THREAD_PRIORITY_HIGHEST` (`s_PriorityBoosted`).
- **Loop** (`FinalizerThreadWorkerIteration`, ~542): (1) espera evento; (2) processa itens de
  trabalho pendentes do GC; (3) `JitHost::Reclaim()` e trabalho extra do runtime
  (`DoExtraWorkForFinalizer`, `DelayDestroyDynamicMethodDesc`); (4) `FinalizeAllObjects()`;
  (5) `SignalFinalizationDone(observedFullGcCount)` que acorda quem espera drenar a fila.
- `FinalizeAllObjects` chama **código gerenciado**: `GC.RunFinalizers` (`[UnmanagedCallersOnly]`,
  `GC.CoreCLR.cs:342`), um loop `GetNextFinalizableObject` -> chama o `Finalize` via ponteiro de
  função -> `catch` de exceção tratada pelo handler global -> `ResetFinalizerThread()`.
  Eventos ETW: `GCFinalizersBegin/End`.
- **Falha de finalizador** (exceção não tratada) é entregue ao handler global; o loop continua.
- **Desligamento**: `RaiseShutdownEvents` ativa `fQuitFinalizer` e espera
  `hEventFinalizerToShutDown`; **em WASM não há thread de finalizador** (comentários:
  "finalizers queued at process exit are not pumped and leak by design").
- `GC.WaitForPendingFinalizers()` (`GC.CoreCLR.cs:375`) bloqueia até a fila ser drenada
  (QCall `GCInterface_WaitForPendingFinalizers`).

### APIs gerenciadas relacionadas (`GC.CoreCLR.cs`)

- `GC.SuppressFinalize(obj)` -> `SuppressFinalizeInternal`: **[inferido]** seta o bit no header do objeto
  (`BIT_SBLK_FINALIZER_RUN`, que `ScanForFinalization` testa; não li a implementação nativa). Em `MemoryManager<T>.IDisposable.Dispose` o framework
  chama `GC.SuppressFinalize(this)` (arquivo 08).
- `GC.ReRegisterForFinalize(obj)` -> `GCInterface_ReRegisterForFinalize`: re-inclui na fila
  (usado em ressurreição).
- `GC.WaitForPendingFinalizers()`.

## Por quê

- **Liberar recursos não gerenciados** (handles de SO, memória nativa) cuja vida é atrelada a
  objetos gerenciados, mesmo que o programador esqueça de chamar `Dispose`.
- **Rodar o finalizador numa thread própria** evita executar código arbitrário do usuário
  *dentro* do GC (com o mundo parado): o GC só **enfileira**. Essa separação é a decisão
  central do mecanismo. Os comentários mostram também que o GC chama o EE por interface
  (`GCToEEInterface::EnableFinalization`).
- **Fila segmentada por geração** acompanha a hipótese geracional: numa coleta de gen0 só se
  varre o segmento de gen0, em vez de toda a fila.
- **Duas listas (comum e crítica)** garantem que finalizadores críticos (como os de
  `SafeHandle`) rodem **depois** dos comuns.
- **Prioridade alta e espera em duas etapas** (2 s, depois low-memory) para o finalizador ser
  responsivo sob pouca memória.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Finalizador roda em outra thread, depois do GC | GC curto e seguro | Ordem e momento imprevisíveis; concorrência com o código do app |
| Objeto com finalizador sobrevive pelo menos uma coleta extra (ressurreição temporária) | Permite acessar o objeto no finalizador | Objeto e **tudo que ele referencia** são promovidos; sobrevive a gen mais velha; pressão no GC |
| Registrar na alocação (`GC_ALLOC_FINALIZE`) | Simples | Alocar objeto finalizável é mais caro (lock da fila `CFinalize`); sem alocação "rápida" em pilha: JIT não aloca finalizáveis na pilha (arquivo 09) |
| Uma única thread de finalizador | Simplicidade | Finalizador lento ou bloqueado trava a fila inteira; vazamento de memória possível |
| Finalizador pode ressuscitar | Flexibilidade | Complexidade (`ReRegisterForFinalize`, bits no header, fila "FReachable") |
| Fila no GC por geração | Eficiência | Código de mover itens entre segmentos, relocação, merge/split por heap |
| Sem garantia de execução no shutdown (WASM, e outros casos) | Portabilidade | Programadores não podem depender do finalizador |
| Prioridade `HIGHEST` na thread | Responsividade | Pode competir com threads do app |

## Como aplicar na sua linguagem

1. **Questione se a sua linguagem precisa de finalizadores.** Alternativas mais previsíveis:
   `defer`/RAII/`using`, ownership explícito, handles com contagem de referência, e `Dispose`
   determinístico. O .NET mantém finalizadores como *rede de segurança*; o modelo recomendado
   é `Dispose` + `SuppressFinalize` (veja o padrão em `MemoryManager<T>`).
2. **Se oferecer finalização, nunca execute-a no GC.** Enfileire e deixe uma thread dedicada
   executar. Preveja: um evento de acordar, um evento de "drenado" (para
   `WaitForPendingFinalizers`), e um evento de shutdown.
3. **Registre objetos finalizáveis na alocação** com uma flag (`GC_ALLOC_FINALIZE`) e mantenha
   uma estrutura separada de rastreamento **fora** do conjunto de raízes.
4. **Particione a fila por geração** (segmentos de uma única matriz com ponteiros de
   preenchimento) se tiver GC geracional.
5. **Defina a semântica de ressurreição**: o objeto e seus alcançáveis são promovidos
   enquanto na lista "f-reachable". Decida se o finalizador roda uma vez só (`FINALIZER_RUN`
   bit) e se permite re-registrar.
6. **Separe finalizadores críticos** (para recursos de SO) e rode-os por último.
7. **Trate exceções em finalizadores** no loop da thread (o .NET entrega ao handler global e
   continua); caso contrário um erro derruba a fila.
8. **Decida a política de shutdown**: o .NET tenta drenar no shutdown em plataformas com
   thread, mas não em WASM.
9. **Evite finalizadores em objetos pequenos/frequentes**: eles impedem otimizações como
   stack allocation e aumentam custo de coleta. Considere restringir finalizadores a
   classes especiais (por exemplo, `SafeHandle`-like).
10. **Dê visibilidade**: eventos `FinalizersBegin/End` e o contador de objetos na fila
    (`GetNumberFinalizableObjects`) ajudam a diagnosticar atrasos.

## Referências

- `src/coreclr/gc/finalization.cpp` - `CFinalize::Initialize`, `RegisterForFinalization`
  (172), `GetNextFinalizableObject` (~233), `MoveItem`, `GcScanRoots`, `ScanForFinalization`
  (~340), `RelocateFinalizationData`, `MergeFinalizationData`, `SplitFinalizationData`,
  `schedule_finalizer_work` (13).
- `src/coreclr/gc/gcpriv.h` - classe `CFinalize` (5688), `finalize_queue` (3916).
- `src/coreclr/gc/interface.cpp` - `GCHeap::RegisterForFinalization` (2496).
- `src/coreclr/gc/gcinterface.h` - `GC_ALLOC_FINALIZE`.
- `src/coreclr/vm/finalizerthread.cpp` - `FinalizerThreadStart`,
  `FinalizerThreadWorkerIteration`, `WaitForFinalizerEvent`, `FinalizeAllObjects`,
  `RaiseShutdownEvents`.
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` - `RunFinalizers`,
  `SuppressFinalize`, `ReRegisterForFinalize`, `WaitForPendingFinalizers`.
- `docs/design/coreclr/botr/garbage-collection.md` - menciona que a thread de finalizador
  recebe notificação assíncrona de pouca memória, que dispara o coletor.
- `src/coreclr/vm/jitinterface.cpp:1877` - `canAllocateOnStack` exclui tipos finalizáveis.
