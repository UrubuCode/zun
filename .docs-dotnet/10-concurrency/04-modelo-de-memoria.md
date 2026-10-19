# Modelo de memória (volatile, barreiras, ECMA versus CLR)

Fonte primária: `docs/design/specs/Memory-model.md` (300 linhas), mais as primitivas em
`src/libraries/System.Private.CoreLib/src/System/Threading/Volatile.cs` e `Interlocked.cs`.

## Mecanismo no .NET

### ECMA-335 versus o modelo "prático" do .NET

O documento afirma que o ECMA-335 define um modelo de memória **muito fraco**, e que, após duas décadas, o
desejo de ter um modelo flexível não trouxe benefícios consideráveis porque o hardware é mais estrito;
programar contra o modelo ECMA exige complexidade extra. As implementações do runtime convergiram para um
modelo que é "um compromisso prático entre o que se implementa eficientemente no hardware atual e o que é
razoavelmente acessível ao desenvolvedor". O texto descreve esse modelo; ele é a referência para compiladores
(JIT) e bibliotecas.

### Alinhamento e atomicidade

- Variáveis primitivas gerenciadas são alinhadas ao seu tamanho (1, 2, 4 bytes; 8 bytes em 64 bits;
  `native int`/ponteiros ao tamanho do ponteiro). Exceção: `FieldOffsetAttribute` explícito.
- Acessos a dados primitivos e enums **alinhados**, de tamanho até o do ponteiro, são **atômicos**: o valor
  observado é sempre resultado de leituras/escritas completas. Referências gerenciadas são sempre atômicas.
- `Interlocked.*` e `Volatile.*` são atômicos independentemente da plataforma — exemplo do documento:
  `Volatile.Read<double>` numa plataforma de 32 bits é atômico, uma leitura normal pode não ser.
- Memória não gerenciada: sem garantias; `unaligned.` e `Unsafe.ReadUnaligned/WriteUnaligned` evitam falhas mas
  **não** garantem atomicidade.

### Efeitos colaterais e otimizações permitidas

O runtime assume que leituras/escritas só observam/alteram valores em locais de memória (diferente do ECMA):

- **Escritas especulativas não são permitidas.** **Leituras não podem ser introduzidas.**
- Leituras não usadas podem ser removidas (se a leitura puder causar falha, não é "não usada").
- Leituras adjacentes não-voláteis do mesmo local podem ser **fundidas**; escritas adjacentes idem.
- Motivação declarada: escrita especulativa mudaria valor observável; reler poderia buscar outro valor e
  introduzir uma corrida que o programa não tinha; fundir acessos *remove* corridas, o que é aceitável.
  Programas que dependem de observar corridas devem usar acessos voláteis.
- Se o compilador prova que dados são acessíveis a uma só thread, pode duplicar/remover acessos.

### Ordem das operações

| Categoria | Semântica | Exemplos no documento |
|---|---|---|
| Acessos comuns | Podem ser reordenados desde que preserve consistência single-thread (compilador ou hardware) | leituras/escritas normais |
| Leitura volátil | **Acquire**: nenhuma leitura/escrita posterior pode ser executada antes | prefixo IL `volatile.` em loads; `Volatile.Read`; `Thread.VolatileRead`; `Volatile.ReadBarrier`; adquirir lock (`Monitor.Enter`, método sincronizado) |
| Escrita volátil | **Release**: efeitos só ficam visíveis depois dos efeitos de todas as leituras/escritas anteriores | `volatile.` em stores; `Volatile.Write`; `Thread.VolatileWrite`; `Volatile.WriteBarrier`; liberar lock (`Monitor.Exit`) |
| Full fence | Efeitos de leituras/escritas observáveis nem antes nem depois da operação | `Thread.MemoryBarrier`; **todos os métodos de `Interlocked`** |
| `volatile. initblk` | Release | |
| `volatile. cpblk` | Combina leitura+escrita voláteis; granularidade interna não especificada | |

Notas do documento: *volatile não implica atomicidade* nem "rapidez" de propagação; só especifica a ordem
quando os efeitos se tornam observáveis.

**Palavra-chave `volatile` em C#:** "declarar um campo `volatile` não tem efeito sobre como o runtime trata o
campo"; é uma dica ao compilador C# (e compiladores de outras linguagens) que emite acessos voláteis.

### Barreira de processo inteiro

`Interlocked.MemoryBarrierProcessWide()` (`Interlocked.cs`: `LibraryImport` para
`Interlocked_MemoryBarrierProcessWide`, não disponível em Mono): full fence mais garantia de que **cada
thread** do programa executa efetivamente um full fence em algum ponto sincronizado — efeito de escritas
anteriores às duas barreiras fica visível. Documento sugere implementação possível: interromper cada
core do affinity mask do processo. Útil para algoritmos assimétricos (lado raro paga o custo).

### Outras regras

- **Métodos sincronizados** (`MethodImplOptions.Synchronized`): mesma semântica de memória que lock
  na entrada e liberação na saída.
- **Leituras dependentes de dados são ordenadas:** ao ler de uma localização derivada de uma referência, a
  leitura não ocorre antes de obter a referência (exemplo no documento sobre `nonlocal.a.b`). Isso vale
  para referências gerenciadas e não gerenciadas.
- **Atribuição de objeto** a local potencialmente compartilhado é **release** em relação aos campos/elementos
  e metadados da instância (construção + publicação é segura sem fences). O leitor não precisa de leitura
  acquire graças à ordenação de leituras dependentes. O documento avisa para **não usar** isso como ordenação
  geral: atribuições independentes podem ser reordenadas e o compilador pode omitir o release se provar que o
  objeto não é compartilhado. Também diz que investigações adicionais são necessárias para garantir que
  otimizações do compilador respeitam essas garantias.
- **Construtores de instância:** nenhum efeito de ordenação é especificado.
- **Construtores estáticos:** todos os efeitos tornam-se observáveis antes de qualquer acesso a membro do tipo.
- **Acesso a locais de outra thread:** não há mecanismo type-safe; via código unsafe é comportamento
  indefinido.

### Expectativas de hardware

O runtime exige: acessos alinhados até o tamanho do ponteiro atômicos (inclusive com sobreposição de tamanhos
diferentes); cache coerente e **multicopy atomic** (escritas a um local vistas na mesma ordem por todos os
cores); store-buffer forwarding permitido; memória "comum" (não registradores de dispositivo); possibilidade de
implementar *release consistency* (x64 é TSO, mais forte; outras plataformas via fences); ordenação de leituras
dependentes (todas as plataformas suportadas honram dependência por padrão).

### Exemplos canônicos (no documento)

1. Construir instância e compartilhar com outra thread: seguro sem fences (leitura única em local, sem releitura
   introduzida, publicação release, leituras dependentes).
2. Singleton com `lock` (double-checked): lock = acquire/release.
3. Singleton com `Interlocked.CompareExchange`: pode construir várias instâncias, mas só uma vence; CAS é
   full fence.
4. Flag de comunicação: `while (!Volatile.Read(ref flag))` — sem `Volatile`, as leituras poderiam ser fundidas
   em uma só fora do laço.

### Como a biblioteca usa isso (exemplos reais)

- `ThreadPoolWorkQueue.WorkStealingQueue`: usa `Interlocked.Exchange` em `m_tailIndex`/`m_headIndex` como
  "write + full fence" (comentário: "decrement the tail using a fence to ensure subsequent read doesn't come
  before") e `Volatile.Read/Write` nos slots do array.
- `ConcurrentQueueSegment`: `SequenceNumber` por slot lido com `Volatile.Read`, publicado com `Volatile.Write`.
- `ConcurrentDictionary`: lê o campo `_tables` volátil uma vez em local; `Volatile.Write` ao publicar nós.
- `ManualResetValueTaskSourceCore`: `Volatile.Read(ref _continuation)` e CAS.
- `Volatile.cs` comenta que o runtime pode substituir as implementações por mais eficientes (ex.: no
  importer do JIT); métodos marcados `[Intrinsic]`.

## Por quê

- **Previsibilidade versus desempenho:** o modelo ECMA fraco impediria programas portáveis de raciocinar sobre
  concorrência sem barreiras excessivas; o modelo prático é fraco o bastante para permitir otimizações do JIT
  (fundir/remover acessos) e fiel ao hardware real (x64 TSO, ARM64 com release/acquire).
- **Proibir introdução de leituras/escritas** evita "corridas inventadas" pelo compilador — o que mantém
  programas sem corridas de dados corretos.
- **Publicação de objetos segura por padrão** reduz uma classe inteira de bugs (objeto visto meio construído),
  um problema histórico do double-checked locking em outros ambientes.
- **Primitivas explícitas** (`Volatile`, `Interlocked`, `Monitor`) tornam a intenção legível e permitem que o
  runtime implemente cada uma com a instrução ótima por arquitetura.

## Trade-offs

- Modelo ainda **mais fraco que sequentially consistent**: `Volatile.Write` seguido de `Volatile.Read` de
  outro local pode ser reordenado (acquire/release não é SC); algoritmos tipo Dekker precisam de full fence.
  (Dedução direta das definições acquire/release do documento.)
- Em x64 muitos bugs de ordenação não aparecem e surgem em ARM64: testar em hardware fraco é necessário.
- O documento admite lacunas (por exemplo, "investigações adicionais necessárias" para garantir que otimizações
  respeitam leituras dependentes/atribuição de objeto).
- Sem suporte a memória incoerente ou de dispositivo: passar tal memória ao runtime é UB.
- `volatile` do C# é só dica de compilador: portabilidade entre linguagens depende de cada uma emitir o prefixo
  `volatile.` corretamente.
- Leitura de `double`/`long` em 32 bits não-atômica sem `Volatile`/`Interlocked`.

## Como aplicar na sua linguagem

1. **Escreva o modelo de memória como documento normativo** antes de otimizar — o `Memory-model.md` do .NET é
   um bom esqueleto: alinhamento, atomicidade, ordem, o que o compilador pode e não pode fazer.
2. **Proíba leituras/escritas introduzidas pelo compilador**; permita fusão e remoção. Documente a motivação.
3. **Escolha SC-DRF (sequencial consistente para programas sem corrida) ou acquire/release.** O .NET ficou com
   acquire/release + full fence em operações atômicas. Se quiser simplicidade, atômicos SC por padrão e opções
   relaxadas para especialistas (isto é um design recomendado, não o do .NET).
4. **Garanta publicação segura de objetos recém-construídos** (release na atribuição + leituras dependentes
   ordenadas) para eliminar bugs de inicialização.
5. **Dê nomes a operações atômicas e de barreira** em vez de depender de uma palavra-chave `volatile` só de
   compilador; torne o significado da palavra-chave idêntico em todas as linguagens do seu runtime.
6. **Defina o que é atômico**: leituras alinhadas de até o tamanho do ponteiro. Para tipos maiores, exija
   `Atomic<T>` explícito.
7. **Considere barreira de processo inteiro** se pretende oferecer estruturas assimétricas (lado rápido sem fence).
8. **Documente garantias de construtores estáticos/inicializadores** (visibilidade antes do primeiro acesso).
9. **Teste em ARM e x64**; use ferramentas de stress e, se possível, um verificador de modelo.

## Referências

- `docs/design/specs/Memory-model.md`
- `docs/design/coreclr/botr/clr-abi.md` (menção sobre atomicidade de `memset`/`memcpy` e ponteiros gerenciados)
- `src/libraries/System.Private.CoreLib/src/System/Threading/Volatile.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Interlocked.cs` (`MemoryBarrier`, `MemoryBarrierProcessWide`)
- `src/libraries/System.Private.CoreLib/src/System/Threading/ThreadPoolWorkQueue.cs` (uso em `WorkStealingQueue`)
- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentQueueSegment.cs`
- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentDictionary.cs`
