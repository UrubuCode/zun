# Finalização, fila de finalizadores e IDisposable

## Mecanismo no .NET

### Finalizador

Em C#, `~Foo() { ... }` é compilado para um método virtual `Finalize` (override de
`object.Finalize`) com try/finally que chama `base.Finalize()`. O runtime trata tipos cujo
`Finalize` é diferente do vazio de `object` como **finalizáveis** (flag no `MethodTable`).

### Registro na alocação

Ao alocar um objeto finalizável o runtime passa `GC_ALLOC_FINALIZE`
(`src/coreclr/gc/gcinterface.h`, ~linha 1094) ao GC. O GC registra o objeto em sua fila de
finalização: `CFinalize::RegisterForFinalization(int gen, Object* obj, size_t size)` em
`src/coreclr/gc/finalization.cpp` (linha ~172). A classe `CFinalize` (declarada em
`gcpriv.h` ~linha 5688) mantém um array segmentado:

- Segmentos delimitados por ponteiros de preenchimento (`m_FillPointers`, `SegQueue`,
  `SegQueueLimit` — usados em finalization.cpp);
- Uma região por geração dos objetos registrados (para que a coleta de gen0 só examine os
  finalizáveis de gen0);
- `FinalizerListSeg`/`CriticalFinalizerListSeg`: objetos cujo "alcançabilidade" acabou,
  prontos para finalizar ("f‑reachable queue");
- `FreeListSeg`: slots livres.

Consequência: alocar um objeto finalizável é **mais lento** (usa helper de alocação mais
lento que `CORINFO_HELP_NEWSFAST`, ver comentário "non-finalizer" em `corinfo.h` ~linha 350) e
o objeto tem custo extra no GC.

### Ciclo de vida

1. Objeto vivo e registrado.
2. Durante um GC, se o objeto **não é alcançável** a partir de raízes (exceto a própria fila
   de finalização), o GC o move da lista de "registrados" para a fila de "prontos para
   finalizar" e **o ressuscita**: marca‑o e tudo que ele referencia como vivos (por isso os
   sobreviventes são promovidos de geração — pagam pelo menos uma coleta extra).
3. A **thread finalizadora** (única, dedicada) acorda — `src/coreclr/vm/finalizerthread.cpp`:
   `FinalizerThread::FinalizeAllObjects` (linha ~377) e
   `FinalizerThread::GetNextFinalizableObject` (linha ~303) retiram objetos de
   `CFinalize::GetNextFinalizableObject` (finalization.cpp linha ~233) e chamam `Finalize`.
   Objetos "críticos" (`CriticalFinalizerObject`, ex. `SafeHandle`) são finalizados depois
   dos normais.
4. Após o `Finalize`, o objeto não está mais registrado; a **próxima** GC que o encontrar
   inalcançável o reclama de fato. (Ressurreição possível: `GC.ReRegisterForFinalize`.)
5. `GC.SuppressFinalize(obj)` (`GC.CoreCLR.cs` ~linha 386, que chama a intrínseca do VM;
   no VM `GCInterface::SuppressFinalize` em `comutilnative.cpp` ~linha 1014) marca o bit
   `BIT_SBLK_FINALIZER_RUN` no header do objeto (ver arquivo 02), fazendo o GC ignorar a
   finalização.

Garantias **fracas**: não há ordem entre finalizadores de objetos relacionados; não há
garantia de que rodem no encerramento do processo (o .NET Core não garante finalização no
shutdown; ver detalhes adicionais); rodam em thread
própria, o que pode gerar corrida com código do usuário.

### IDisposable e o padrão Dispose

`IDisposable.Dispose()` é uma convenção de **liberação determinística**, não um recurso do
runtime. O compilador adiciona açúcar: `using (var x = ...)` → `try { } finally { x.Dispose(); }`
(`using var`/`await using` em C# 8; `IAsyncDisposable`).

Padrão canônico:

```csharp
class Recurso : IDisposable {
    private bool _disposed;
    public void Dispose() { Dispose(true); GC.SuppressFinalize(this); }
    protected virtual void Dispose(bool disposing) {
        if (_disposed) return;
        if (disposing) { /* liberar recursos gerenciados */ }
        /* liberar recursos nativos */
        _disposed = true;
    }
    ~Recurso() => Dispose(false);
}
```

Implementação real: `SafeHandle` em
`src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/SafeHandle.cs`
(deriva de `CriticalFinalizerObject`, tem `~SafeHandle()` na linha ~79 e
`Dispose(bool disposing)` na ~105), com contagem de referência para impedir fechar o handle
enquanto uma chamada P/Invoke o usa.

`GC.SuppressFinalize` no `Dispose` evita o custo da finalização (e a promoção extra).

### Alternativas modernas

`SafeHandle` (preferível a finalizadores à mão), `ConditionalWeakTable`,
`DependentHandle`, `WeakReference`, e `GCHandle`; o .NET também tem *finalization sequencing
via `CriticalFinalizerObject`*. Não há mais Finalizer em nada gerenciado comum: a BCL moderna
evita finalizadores.

## Por quê

- Memória e outros recursos têm ciclos diferentes: o GC só vê pressão de **memória**;
  arquivos, sockets, handles nativos precisam de liberação previsível → `IDisposable`.
- Finalizador é a **rede de segurança** quando o usuário esquece de chamar `Dispose`.
- Ressuscitar o objeto para finalizar permite que o finalizador acesse campos (para fechar
  o handle), ao custo da promoção.
- Thread dedicada isola código de usuário arbitrário do GC (nunca roda código do usuário
  dentro da pausa).

## Trade-offs

- Finalizáveis atrasam a recuperação (precisam de ≥ 2 GCs) e sobrecarregam gen1/gen2.
- Não determinístico, sem ordem, thread diferente, pode rodar em objeto parcialmente
  construído (ctor lançou) — fonte de bugs.
- `IDisposable` é só convenção: esquecer `using` vaza; Dispose duplo e uso após Dispose
  exigem `ObjectDisposedException` manual.
- Hierarquias de Dispose são verbosas (padrão `Dispose(bool)`).
- A fila de finalização pode ficar para trás (finalizer starvation) e segurar muita memória.

## Como aplicar na sua linguagem

1. **Separe** gestão de memória de gestão de recursos: forneça construto determinístico
   (`using`/`defer`/RAII/`with`) para recursos não‑memória, e use finalizadores apenas como
   rede de segurança, se existirem.
2. Se oferecer finalizador: registre na **alocação** (como o .NET) só objetos que o tenham,
   com helper de alocação separado para manter o fast path barato.
3. Rode finalizadores em thread própria; nunca na pausa do GC.
4. Defina semântica de ressurreição e de *suppress* (bit no header ou campo).
5. Ofereça um tipo de **handle seguro** na biblioteca padrão (contagem de uso + fechamento
   idempotente) e desencoraje finalizadores escritos à mão.
6. Considere **ownership/move semantics** (Rust `Drop`, C++ RAII) se quiser destrutores
   determinísticos sem GC; `IDisposable` de modo estático (analisadores que checam
   não‑descartados) como compromisso.
7. Para linguagens async, preveja `AsyncDisposable`.

## Detalhes adicionais

### Linha do tempo de um objeto finalizável

```
t0  new Foo()               -> alocação lenta + registro em CFinalize (gen0)
t1  Foo fica inalcançável
t2  GC gen0: detecta, move para f-reachable queue, ressuscita, promove (gen1)
t3  Thread finalizadora roda ~Foo()
t4  GC gen1 (próximo): objeto inalcançável e sem registro -> memória recuperada
```

Se `Dispose()` rodou antes com `SuppressFinalize`, t2/t3 são pulados: o objeto morre no
primeiro GC como qualquer outro.

### Notas

- A thread finalizadora é criada por `FinalizerThread::FinalizerThreadCreate`
  (`finalizerthread.cpp` linha ~771) e espera um evento (`WaitForFinalizerEvent`, ~416)
  sinalizado quando há trabalho. `SignalFinalizationDone` (~825) e `FinalizerThreadWait`
  (~843) suportam `GC.WaitForPendingFinalizers`.
- Sobre o encerramento do processo: não afirmo o comportamento exato; ver
  `FinalizerThread::RaiseShutdownEvents` (~395) antes de decidir para sua linguagem.
- `WeakReference`, `GCHandle` (weak/long weak) e `ConditionalWeakTable` interagem com a
  finalização: handles "short weak" são zerados antes da finalização; "long weak" depois
  (comportamento documentado da API, não verificado no código aqui).

### Alternativas de design em outras linguagens (conhecimento geral)

| Linguagem | Mecanismo |
|---|---|
| Java | `Cleaner`/`PhantomReference` (`finalize` deprecated) |
| Go | `runtime.SetFinalizer` + `defer` |
| Python | `__del__` + `with` |
| Rust | `Drop` determinístico |

## Referências

- `src/coreclr/gc/finalization.cpp` — `CFinalize::RegisterForFinalization`,
  `GetNextFinalizableObject`, segmentos de fila.
- `src/coreclr/gc/gcpriv.h` (~linha 5688, classe `CFinalize`).
- `src/coreclr/vm/finalizerthread.cpp` — `FinalizerThread::FinalizeAllObjects`,
  `GetNextFinalizableObject`.
- `src/coreclr/vm/syncblk.h` — `BIT_SBLK_FINALIZER_RUN`.
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` — `SuppressFinalize`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/SafeHandle.cs`.
- `docs/design/coreclr/botr/garbage-collection.md` (menção a finalizer thread).
