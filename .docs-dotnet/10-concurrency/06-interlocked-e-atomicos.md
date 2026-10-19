# Interlocked e operações atômicas

`System.Threading.Interlocked` é a base de toda estrutura lock-free do .NET: contadores, flags de estado,
publicação de ponteiros, filas e pilhas. Este documento descreve o que existe, como é implementado e como a
biblioteca usa essas operações.

## Mecanismo no .NET

### API (`Interlocked.cs`, `Interlocked.CoreCLR.cs`)

`public static partial class Interlocked` oferece, para `int`, `uint`, `long`, `ulong`, `nint`, `nuint`,
`float`, `double`, `byte`, `sbyte`, `short`, `ushort`, referência/objeto e `T` genérico:

| Operação | Observação confirmada no código |
|---|---|
| `Increment`/`Decrement` | Implementados como `Add(ref x, 1)` / `Add(ref x, -1)` |
| `Add` | Retorna o novo valor |
| `Exchange` | Retorna o valor original |
| `CompareExchange` | `(location, value, comparand)`; retorna o valor original; sucesso = retorno == comparand |
| `And` / `Or` | Retornam o **valor original**; fallback é laço de CAS (`CompareExchange` até estabilizar) |
| `Read` (64 bits) | Para `ulong`: `CompareExchange(ref x, 0, 0)` — leitura atômica emulada com CAS |
| `MemoryBarrier` | Full fence |
| `MemoryBarrierProcessWide` | Barreira de processo inteiro (não em Mono) |

Marcadas `[Intrinsic]`: o JIT substitui por instruções nativas.

### Intrínsecos por arquitetura e emulação

Em `Interlocked.cs`, trechos como:

```
#if TARGET_X86 || TARGET_AMD64 || TARGET_ARM64 || TARGET_RISCV64
    return Exchange(ref location1, value); // Must expand intrinsic
#else
    ... Exchange32(ref location1, value) // chamada ao runtime
#endif
```

mostram que em arquiteturas "principais" o JIT expande a instrução; nas demais há chamadas ao runtime
(helpers). Para `byte` e `ushort`/`short`, quando o alvo não tem instrução nativa, o código **emula via CAS na
palavra de 32 bits alinhada** que contém o byte: calcula `offset` com `Unsafe.OpportunisticMisalignment`,
monta máscara e valor deslocado, e faz laço `CompareExchange` — comentário: "isso depende de o GC manter
alinhamento de 4 bytes para refs e de subtrair até esse alinhamento permanecer no mesmo objeto".

Versões genéricas `Exchange<T>`/`CompareExchange<T>`: para tipos referência delegam à versão `object`; para
valores só aceitam primitivos e enums (tamanhos 1, 2, 4, 8 via versões inteiras); outros lançam
`NotSupportedException`.

### Semântica de memória

Segundo `docs/design/specs/Memory-model.md`: **todos os métodos `Interlocked` têm semântica de full fence** e
são atômicos independente da plataforma. Isso é usado deliberadamente: no `WorkStealingQueue`,
`Interlocked.Exchange(ref m_tailIndex, tail)` é a forma de "escrever e impedir que a leitura seguinte suba",
em vez de escrita volátil + `MemoryBarrier`.

### Padrões de uso reais na base de código

1. **Máquina de estado em um inteiro** — `Task.AtomicStateUpdate` (laço CAS que só liga bits se bits proibidos
   estiverem desligados) e `Interlocked.And(ref m_stateFlags, ~bit)` para limpar flags.
2. **Transição de "não pronto → pronto" com sentinela** — `Task.FinishContinuations` troca
   `m_continuationObject` por um objeto sentinela com `Interlocked.Exchange`; quem adiciona continuação faz
   CAS e, se encontra o sentinela, executa direto.
3. **Contadores empacotados** — `PortableThreadPool.ThreadCounts`: três contadores de 16 bits num `ulong`,
   atualizados em conjunto por CAS (`InterlockedCompareExchange` em `_separated.counts`).
4. **Copy-on-write de array** — `WorkStealingQueueList.Add/Remove`: cria novo array e publica com
   `Interlocked.CompareExchange(ref s_queues, newQueues, oldQueues)`, repetindo se perder a corrida.
5. **Pilha lock-free (Treiber)** — `ConcurrentStack.Push`: `newNode._next = _head; CompareExchange(ref _head,
   newNode, newNode._next)`; se falhar, vai para `PushCore` com backoff ("add some back off to statistically
   decrease contention at the head").
6. **Fila MPMC limitada por sequência** — `ConcurrentQueueSegment`: cada slot tem `SequenceNumber`; `Head`/`Tail`
   avançam por CAS; o número de sequência (Volatile.Write) publica a transição "cheio/vazio" (doc 07).
7. **Reserva de conclusão (primeiro vence)** — `Channels.AsyncOperation`: `Interlocked.Exchange(ref
   _completionReserved, 1) == 0` decide quem completa (resultado versus cancelamento); `_continuation` recebe
   CAS de `null` para `s_completedSentinel`.
8. **Inicialização preguiçosa publicada com CAS** — `CancellationTokenSource`: `Interlocked.CompareExchange(ref
   _registrations, new Registrations(...), null) ?? created`; `_kernelEvent` e `_timer` idem.
9. **Sinalização de uma única execução** — `QueueUserWorkItemCallbackBase` em debug assere
   `!Interlocked.Exchange(ref _executed, true)`.
10. **Flag "requisição pendente"** — `_hasOutstandingThreadRequest` via `Interlocked.Exchange(..., 1)`.

### Classes relacionadas

- `LazyInitializer`, `LazyThreadSafetyMode`: inicialização única com CAS.
- `ThreadInt64PersistentCounter`: contador por thread (somado sob demanda) para estatísticas do pool sem
  contenção em um único contador global.
- `Volatile` (doc 04) para leituras/escritas com acquire/release sem o custo de full fence.
- `SpinWait`/`Backoff.cs`: política de espera para laços de CAS contendidos.

## Por quê

- **Progresso sem locks**: um CAS é a base de algoritmos lock-free/wait-free; evita convoy de locks, inversão de
  prioridade e custo de bloqueio.
- **Full fence por padrão em atômicos**: simplifica raciocínio. O custo extra em ARM é aceito em troca de
  menor risco; leituras/escritas mais baratas existem em `Volatile`.
- **Intrínsecos**: no x64 `Interlocked.Increment` vira `lock xadd`; chamar função seria muito mais lento.
- **Cobertura de tipos pequenos e genéricos**: facilita flags e enums sem exigir que o autor empacote em `int`.
- **`And`/`Or` com retorno do valor original**: permite "set e descobrir se já estava setado" em um passo
  (usado para flags em `Task`).

## Trade-offs

- **ABA**: CAS sobre ponteiros sem contador de versão é vulnerável ao problema ABA em estruturas que reusam
  nós. Em .NET o GC reduz o risco (um nó referenciado não é reciclado), o que é uma vantagem de ter GC;
  em linguagem sem GC é preciso hazard pointers/epochs. (Conclusão sobre vantagem do GC é dedução, não
  citada no código.)
- **Contenção**: laços de CAS com muitos competidores degradam; a base de código usa backoff
  (`ConcurrentStack`) e contadores particionados (`ThreadInt64PersistentCounter`).
- **Full fence caro** em hardware fraco, e inútil quando só se quer ordenação acquire/release.
- **`Read` de 64 bits via CAS** em `ulong` é escrita disfarçada de leitura (causa tráfego de cache-line em
  modo exclusivo).
- **Emulação de byte/short via CAS na palavra de 32 bits** é correta, mas falha em disputa com vizinhos de
  campo e exige alinhamento do GC.
- **Dificuldade de corretude**: estruturas lock-free exigem raciocínio fino sobre o modelo de memória
  (doc 04); comentários do runtime como "NOTE: this write must not be delayed past our check" mostram o
  nível de cuidado necessário.
- Sem transações multi-palavra: para atualizar vários campos juntos, o runtime recorre a empacotar em um
  `ulong` (limita número/tamanho dos campos) ou a locks.

## Como aplicar na sua linguagem

1. **Ofereça um módulo de atômicos de primeira classe** com `load`, `store`, `exchange`, `compare_exchange`,
   `fetch_add/and/or/...`. Retorne o valor **anterior** (permite saber quem "venceu").
2. **Defina a ordenação de cada operação** (relaxada, acquire, release, seq-cst). O .NET só expõe full
   fence em `Interlocked` e acquire/release em `Volatile`; uma linguagem nova pode expor ordens explícitas
   (decisão de design; não é o que o .NET faz).
3. **Tipo atômico no sistema de tipos** (`Atomic<T>`) para impedir mistura de acessos atômicos e não atômicos
   ao mesmo campo (o .NET depende de disciplina do programador).
4. **Ponha o CAS como intrínseco do compilador** e defina o fallback (laço de CAS para byte/short) para
   alvos sem instrução nativa.
5. **Idiomas de "estado em palavra"**: forneça helpers de bitfield para empacotar contadores em 64 bits,
   como `ThreadCounts`.
6. **Sentinela de conclusão** + `exchange` para padrões "adicionar callback ou rodar imediatamente".
7. **Biblioteca padrão de estruturas lock-free** (pilha, fila MPMC limitada por sequência) para que usuários
   não reimplementem; documente o tratamento de ABA se não houver GC.
8. **Dê suporte a contador particionado por thread/core** para estatísticas.
9. **Documente as garantias de memória** de cada primitiva atomicamente (veja doc 04).

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/Interlocked.cs`
- `src/coreclr/System.Private.CoreLib/src/System/Threading/Interlocked.CoreCLR.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Volatile.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/ThreadPoolWorkQueue.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/PortableThreadPool.ThreadCounts.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Task.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/CancellationTokenSource.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/LazyInitializer.cs`, `ThreadInt64PersistentCounter.cs`
- `src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/ConcurrentStack.cs`
- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentQueueSegment.cs`
- `src/libraries/System.Threading.Channels/src/System/Threading/Channels/AsyncOperation.cs`
- `docs/design/specs/Memory-model.md`
