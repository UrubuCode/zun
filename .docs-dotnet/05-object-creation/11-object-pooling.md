# Object pooling (ArrayPool, ObjectPool) — reuso

## Mecanismo no .NET

### Por que pooling existe

Em um GC geracional, objetos de vida curta e pequenos são baratos (arquivo 06), mas
**buffers grandes** (>= 85.000 bytes, LOH — arquivo 07) são caros: zeragem, orçamento de LOH,
coleta só em gen2. O `ArrayPool<T>` existe para reusar arrays em vez de alocá‑los.

### ArrayPool<T> (neste repositório)

Código em `src/libraries/System.Private.CoreLib/src/System/Buffers/`:

- `ArrayPool.cs` — classe abstrata. `ArrayPool<T>.Shared` (linha ~37) retorna uma instância
  estática `SharedArrayPool<T>` (o comentário em ~linha 22 diz que é campo `static readonly`
  tipado concretamente para permitir devirtualização quando `Shared` é inlinado).
  `ArrayPool<T>.Create()` / `Create(maxArrayLength, maxArraysPerBucket)` criam um
  `ConfigurableArrayPool<T>`. API: `Rent(int minimumLength)` e
  `Return(T[] array, bool clearArray = false)`.
- `Utilities.cs` — `SelectBucketIndex(int bufferSize)`: `BitOperations.Log2((uint)bufferSize
  - 1 | 15) - 3`. Buffers são agrupados em **potências de 2**: um pedido entre 2^(n‑1)+1 e
  2^n recebe um array de tamanho 2^n; 1‑16 compartilham o menor bucket; tamanho 0 recebe o
  índice máximo e o pool devolve `[]` (sem reter).
- `SharedArrayPool.cs`:
  - `NumBuckets = 27` (comentário: `SelectBucketIndex(1024*1024*1024 + 1)`), ou seja, até
    arrays de 1 GiB (2^30 elementos).
  - **Dois níveis**: (1) cache **thread‑local** (`[ThreadStatic] t_tlsBuckets`, um array por
    bucket: `Rent` tenta `tlsBuckets[bucketIndex].Array` e o limpa; (2) **partições
    por‑core** (`SharedArrayPoolPartitions`, em `_buckets`) acessadas por `TryPop`.
  - Se nada está disponível, aloca: `typeof(T).IsPrimitive && typeof(T) != typeof(bool) ?
    GC.AllocateUninitializedArray<T>(minimumLength) : new T[minimumLength]` — **não zera**
    arrays de primitivos (o conteúdo devolvido por `Rent` é arbitrário) e arredonda o
    tamanho ao do bucket para poder devolver depois.
  - `Return(array, clearArray)` devolve ao TLS e, se cheio, às partições; `clearArray`
    zera antes de guardar. Há `Trim()` (linha ~191) que descarta buffers em desuso de
    acordo com a pressão de memória (`Gen2GcCallback`/callback de gen2 e
    `ArrayPoolEventSource.BufferTrimPoll`).
  - Telemetria via `ArrayPoolEventSource` (eventos de rent/return/alocação/trim).
- `ConfigurableArrayPool.cs` — versão com limites configuráveis (`MinimumArrayLength = 0x10`,
  `MaximumArrayLength = 0x40000000`), buckets com lock.

Contrato para o usuário: o array devolvido por `Rent` pode ser **maior** que o pedido;
conteúdo **indefinido**; após `Return`, não usar mais (use‑after‑return é bug silencioso);
o `Return` de array de tamanho não‑bucket é descartado.

### ObjectPool<T> (fora deste repositório)

`Microsoft.Extensions.ObjectPool` (`ObjectPool<T>`, `DefaultObjectPool<T>`,
`IPooledObjectPolicy<T>`, `ObjectPoolProvider`, `StringBuilderPooledObjectPolicy`) pertence
ao repositório `dotnet/aspnetcore` — **não existe em `src/libraries` do dotnet/runtime**
(confirmado: não há pasta `Microsoft.Extensions.ObjectPool`). Por isso não é possível citar
código aqui; o que se sabe do design público: `Get()` / `Return(T)`; uma política cria e
"limpa" (`Create()`, `Return(T) → bool`); o `DefaultObjectPool` guarda um item rápido
(`_fastItem`) + uma fila limitada.

Pools similares no próprio repo:
- `src/libraries/System.Reflection.Metadata/src/System/Reflection/Internal/Utilities/ObjectPool`1.cs`
  — pool interno simples.
- `src/libraries/System.Collections.Immutable/src/System/Collections/Immutable/SecureObjectPool.cs`.
- `ThreadPool` e *work items* reutilizam objetos; `SocketAsyncEventArgs`, `ArrayBufferWriter`,
  `StringBuilderCache` (`src/libraries/Common/src/System/Text/StringBuilderCache.cs`). (Os demais:
  não verificados.)

### Relação com GC

Objetos em pool vivem muito → são promovidos a gen2 e **deixam de ser lixo**; mas
referências de gen2 para gen0 (ex.: pool guardando objetos novos) geram trabalho na write
barrier/card table. Pools devem ser limitados e podados (trim) para não segurar memória.

## Por quê

- Evitar alocação e zeragem de buffers grandes → menos GCs gen2 e menos fragmentação do LOH.
- Buckets de potência de 2 dão trade‑off simples entre desperdício (< 50%) e taxa de acerto.
- Cache por thread + por core minimiza contenção (lock‑free no caso comum).
- `clearArray` opcional: só paga a limpeza quando há dados sensíveis ou referências.
- Trim vinculado à GC alinha o tamanho do pool com a pressão de memória.

## Trade-offs

- **Bugs de ciclo de vida**: use‑after‑return, double‑return, vazamento (não devolver é
  seguro — o GC coleta — mas perde o benefício), dados residuais expostos (segurança).
- Complexidade na API (try/finally por todo lado); `Rent` pode devolver array maior.
- Pools seguram memória (heap maior em idle); promovem a gen2.
- Para objetos pequenos, pooling muitas vezes é **mais lento** que `new` por causa do custo
  de sincronização e da write barrier.
- Dependência de convenções: o runtime não detecta uso indevido.

## Como aplicar na sua linguagem

1. Só ofereça pools na biblioteca padrão para **buffers grandes**; ensine que objetos
   pequenos devem usar `new` (bump pointer — arquivo 06).
2. Projete o `Rent/Return` com *buckets* de potência de 2, TLS primeiro e estrutura
   compartilhada por core depois (cópia do design do `SharedArrayPool`).
3. Documente: conteúdo indefinido, tamanho >= pedido, proibido usar depois de devolver.
   Considere tornar isso verificável: um tipo `Lease`/`IDisposable` que devolve ao fechar
   (`using var buf = pool.Rent(n)`), com Dispose idempotente.
4. Exponha eventos/telemetria (rent, return, alocação por falha, trim) como o
   `ArrayPoolEventSource`.
5. Implemente *trim* acionado por pressão do GC.
6. Ofereça `AllocateUninitialized` para o caminho de miss do pool.
7. Em linguagens com ownership (Rust), o próprio tipo `Vec`/`Box` com `Drop` devolvendo ao
   pool elimina use‑after‑return em compile‑time.
8. Para pool genérico de objetos, adote o modelo `Policy` (Create / Return→bool) para
   permitir limpar o objeto ou descartá‑lo.

## Detalhes adicionais

### Fluxo de Rent/Return do SharedArrayPool (resumo)

```
Rent(n):  bucket = SelectBucketIndex(n)
          1. TLS[bucket]?    -> devolve e limpa o slot
          2. per-core[bucket].TryPop()?  -> devolve
          3. senão aloca array do tamanho do bucket (uninitialized p/ primitivos)
Return(a): bucket pelo tamanho do array; opcional: limpar
          guarda no TLS (se vazio) ou na partição per-core (limitada)
Trim():   registrado via Gen2GcCallback.Register(...) (SharedArrayPool.cs ~linha 296);
          o comentário (~243-244) diz que leva ao menos duas chamadas de Trim (dois GCs
          gen2) para largar um array, exceto sob alta pressão de memória
```

### Pools internos úteis como modelo

- `src/libraries/Common/src/System/Text/StringBuilderCache.cs` — cache de `StringBuilder`.
- `src/libraries/Common/src/System/Text/ValueStringBuilder.cs` e
  `src/libraries/Common/src/System/Collections/Generic/ValueListBuilder.cs` — structs que
  começam em buffer de pilha e crescem para `ArrayPool.Shared`.

### Checklist de projeto de um pool

- Granularidade: buckets por potência de 2 (`Utilities.SelectBucketIndex`).
- Camadas: thread-local, por core, global.
- Política de limpeza: `clearArray` opcional.
- Política de crescimento e de poda (trim) ligada ao GC.
- Telemetria (`ArrayPoolEventSource`).
- Contrato de propriedade: quem pode usar o buffer após `Return`? (ninguém.)
- Tamanho máximo: arrays acima do maior bucket não são retidos pelo pool.


- Uma alternativa a pools é reduzir alocação com Span/stackalloc (arquivo 10) ou reuso de um único buffer por operação.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Buffers/ArrayPool.cs`,
  `SharedArrayPool.cs`, `ConfigurableArrayPool.cs`, `Utilities.cs`, `ArrayPoolEventSource.cs`.
- `src/libraries/System.Reflection.Metadata/src/System/Reflection/Internal/Utilities/ObjectPool`1.cs`.
- `src/libraries/System.Collections.Immutable/src/System/Collections/Immutable/SecureObjectPool.cs`.
- `Microsoft.Extensions.ObjectPool` — repositório `dotnet/aspnetcore` (externo).
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` — `AllocateUninitializedArray`.
- Arquivos 06 e 07 desta pasta.
