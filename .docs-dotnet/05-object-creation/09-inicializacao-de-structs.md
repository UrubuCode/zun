# Inicialização de structs e o valor default

## Mecanismo no .NET

### Value types sempre têm um valor default

Todo value type (struct/enum) tem um valor *all‑zero bits* válido: `default(S)`. É
garantido pelo runtime por zeragem de memória nos três lugares onde structs vivem:

1. **Como campo de objeto/array**: o GC entrega memória zerada (arquivos 06 e 07). `new
   S[10]` produz 10 structs zerados sem chamar construtores.
2. **Como local / na pilha**: o método tem a flag `CORINFO_OPT_INIT_LOCALS` ("zero
   initialize all variables", `src/coreclr/inc/corinfo.h` ~linha 713), definida pelo
   bit `localsinit` do cabeçalho do corpo IL. O C# a emite por padrão; o atributo
   `[SkipLocalsInit]`
   (`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/SkipLocalsInitAttribute.cs`)
   a suprime.
3. **Como valor de retorno / argumento**: o produtor é responsável (o JIT zera retornos
   por buffer oculto quando necessário).

Consequência: **um struct não pode impor invariantes que `default` quebre**; o C# oferece
`new S()` mas qualquer array, campo ou `default` evita seu ctor.

### IL: `initobj` e `newobj` sobre struct

- `initobj <T>` zera o struct apontado por um endereço (usado por `default(S)` e por `new
  S()` sem ctor declarado).
- `newobj S::.ctor(...)`: o compilador reserva um local, passa seu endereço como `this`
  (byref) ao ctor e carrega o resultado. Não há alocação em heap. O JIT elimina cópias
  (promoção de struct/"struct promotion" → campos em registradores; `src/coreclr/jit/`,
  por exemplo `morph.cpp`, `lclvars.cpp`).
- Ctors de struct (pré C# 10) não podiam ser sem parâmetros; C# 10 permite ctor
  sem parâmetros, mas ele **não** é chamado por `default(S)`, arrays ou campos —
  só por `new S()` explícito.
- Antes do C# 11 o ctor de struct precisava atribuir todos os campos (*definite
  assignment*); a partir do C# 11 campos não atribuídos são auto‑default (o compilador
  insere `initobj`/zero).

### Campos `readonly struct`, `ref struct`

- `readonly struct`: todos os campos readonly; o compilador pode passar `in`/cópia
  defensiva menos.
- `ref struct` (`Span<T>`): só vive na pilha — ver arquivo 10. `Span<T>` em
  `src/libraries/System.Private.CoreLib/src/System/Span.cs` (linha 28) é `readonly ref
  struct` com `ref T _reference` + `int _length`; `default(Span<T>)` é um span vazio válido
  (`Span<T>.Empty => default;` linha 222), exemplo de design em que o zero é um estado
  válido por construção.

### Struct dentro de objeto e layout

O layout do struct segue `StructLayout` (Sequential para structs C# por padrão; Auto
quando contém referências). O `GCDesc` do tipo contêiner descreve onde ficam as referências
dentro de structs embutidos (ver arquivo 02), de modo que o GC varre structs em campos e em
arrays de structs sem boxing.

### Boxing

Quando um struct é convertido para `object`/interface, o runtime aloca um objeto no heap com
header + MethodTable + cópia dos campos (o "box"). `unbox` retorna um ponteiro para o dado
dentro do box. Isto reaproveita o mesmo layout de objeto (arquivo 02). `default(T)` de um
`Nullable<T>` é `null` ao ser boxeado (tratamento especial).

### Enum e primitivos

`default(int)` = 0, `default(bool)` = false, enums = 0 mesmo se nenhum membro tem valor 0.

## Por quê

- **Arrays e campos não precisam de passada de construção**: `new S[1_000_000]` custa só
  uma alocação grande + zeragem do GC. Se o valor zero não fosse válido, seria preciso
  rodar um ctor por elemento (como C++ `new T[n]`).
- **Segurança**: nunca há struct "não inicializado" observável; elimina leitura de lixo.
- **Simplicidade do runtime**: um único invariante "memória nova = zero".
- **`initobj`** como instrução própria dá ao JIT um padrão claro para vetorizar zeragem.

## Trade-offs

- Struct não pode garantir invariantes (ex.: um `struct Money` com moeda obrigatória).
  Solução: tratar `default` como valor "vazio" válido (como `ImmutableArray<T>.IsDefault`,
  `Span`), ou usar classe.
- Zeragem de locais (`localsinit`) tem custo em métodos com muitos locais/`stackalloc`
  grandes → `[SkipLocalsInit]` (e é `unsafe`, perigoso).
- Passar structs grandes por valor copia; exige `in`/`ref` para evitar cópias.
- Ctor sem parâmetros em struct que não roda em `default`/array confunde (inconsistência
  semântica).
- Boxing aloca e causa pressão no GC.
- Structs mutáveis em coleções têm semântica de cópia surpreendente.

## Como aplicar na sua linguagem

1. Decida o **invariante de zero**: "todo tipo valor tem representação all‑zero válida".
   Isso permite alocar arrays/campos sem construtores. Swift, Rust e Go divergem (Go
   também adota zero value; Rust exige `Default` explícito e não tem arrays implícitos).
2. Se permitir ctor sem parâmetros em tipos valor, ou **garanta que ele sempre roda**
   (impondo custo em arrays) ou **proíba** (como o C# pré‑10); não deixe ambíguo.
3. Forneça uma primitiva IL/IR de "zerar este endereço" (`initobj`) e uma flag por função
   "zerar locais" (`localsinit`) com possibilidade de desligar em código `unsafe`.
4. Se quiser invariantes fortes, permita `struct` sem valor default (não `default`‑able),
   mas então arrays/campos precisam de inicialização explícita – custo de ergonomia.
5. Implemente *struct promotion* / scalar replacement no otimizador para que structs
   pequenos vivam em registradores.
6. Defina o **boxing** como cópia para um objeto de heap com o mesmo layout; ou proíba
   conversão implícita para evitar alocações escondidas.
7. Projete tipos da biblioteca (Span, Option, handles) de modo que `default` seja um estado
   válido ("vazio").

## Detalhes adicionais

### Exemplo: arrays e `default`

```csharp
struct P { public int X, Y; }
var arr = new P[1000];     // 1 alocação; todos (0,0); nenhum ctor roda
P p = default;             // initobj
P q = new P();             // initobj (sem ctor declarado)
P r = new P { X = 1 };     // initobj + stfld
```

### Retornos e `Unsafe.SkipInit`

`Unsafe.SkipInit<T>(out T value)` (CoreLib) permite declarar que um `out` não será
inicializado, útil com `[SkipLocalsInit]` em código de alto desempenho. Equivale a admitir
lixo na pilha; só vale se tudo for sobrescrito.

### Layout e inicialização parcial

Structs com campos de referência têm o GC descrevendo-os pelo contêiner; um struct `default`
tem referências `null`, seguras para o GC. Um bloco de `localloc` sem `localsinit` contendo
referências seria **inseguro**: por isso o C# só permite `stackalloc` de tipos `unmanaged`
(regra da linguagem).

### Nullable value types

`int?` é `Nullable<int>` (struct com `bool hasValue; int value`); `default` = sem valor.
Boxing de `Nullable<T>` sem valor produz `null`, regra especial da linguagem/runtime.

### Perguntas de projeto

- Tipos valor têm sempre um valor zero válido? Quem garante invariantes?
- `default` e arrays chamam ctor? (.NET: não.)
- Locais são zerados por padrão? Existe opt-out explícito e inseguro?
- Como o boxing é representado e quando é implícito?
- O otimizador promove campos de structs a registradores (scalar replacement)?
- Existe diferença entre struct "vazio" (`default`) e struct "construído"?

## Referências

- `src/coreclr/inc/corinfo.h` — `CORINFO_OPT_INIT_LOCALS` (~linha 713).
- `src/libraries/System.Private.CoreLib/src/System/Span.cs` — `Span<T>`, `Empty`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/SkipLocalsInitAttribute.cs`.
- `src/coreclr/jit/` — promoção de structs, `initobj` no importador (`importer.cpp`).
- `docs/design/coreclr/botr/type-system.md`, `garbage-collection.md`.
- ECMA‑335 §III.4.? `initobj`, `localsinit` (flag `CorILMethod_InitLocals`) — externo ao repo.
