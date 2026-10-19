# Tuplas, ValueTuple e deconstrução

## Mecanismo no .NET

### 1. Duas famílias de tuplas

- `System.Tuple<...>` — **classes** imutáveis (1ª geração; `src/libraries/System.Private.CoreLib/src/System/Tuple.cs`).
- `System.ValueTuple<...>` — **structs** mutáveis (`.../ValueTuple.cs`). É a base das tuplas de C#
  7+ `(int, string)`.

Não há tipo "tupla" no runtime: `(int a, string b)` é açúcar do compilador para
`ValueTuple<int,string>`. O runtime só vê um struct genérico. (Nota em `ValueTuple.cs` ~30:
"their members (such as Item1, Item2, etc) are fields rather than properties".)

### 2. Forma de `ValueTuple`

```csharp
public struct ValueTuple<T1, T2>
    : IEquatable<ValueTuple<T1,T2>>, IStructuralEquatable, IStructuralComparable,
      IComparable, IComparable<ValueTuple<T1,T2>>, IValueTupleInternal, ITuple
{
    public T1 Item1;
    public T2 Item2;
```

- Campos **públicos e mutáveis** (não `readonly`) — decisão deliberada de desempenho/ergonomia.
- Aridades de 0 a 7 diretas: `ValueTuple` (0), `ValueTuple<T1>` ... `ValueTuple<T1..T7>`.
- A 8ª posição é especial: `ValueTuple<T1,...,T7, TRest> where TRest : struct`, com campo `Rest`
  (`ValueTuple.cs` ~1804-1820). Tuplas maiores são **aninhadas**: `(a1,...,a8)` →
  `ValueTuple<A1..A7, ValueTuple<A8>>`. O compilador esconde o aninhamento; `Item8` é
  `Rest.Item1`.
- Implementa `ITuple` (`System.Runtime.CompilerServices.ITuple`: `Length` e indexador `object?
  this[int]`), permitindo acesso uniforme (com boxing) — usado por reflexão/pattern matching.
- Igualdade/ordenação **estrutural** por elemento via `EqualityComparer<T>.Default`.

### 3. Nomes de elementos: só metadado

Os nomes `(int count, string name)` não existem em runtime. O compilador emite
`TupleElementNamesAttribute` (`.../CompilerServices/TupleElementNamesAttribute.cs`, construtor
`TupleElementNamesAttribute(string?[] transformNames)`) em membros/parâmetros/retornos. Assim
`(int a, int b)` e `(int x, int y)` são o **mesmo tipo** em runtime; os nomes são recuperados só por
ferramentas/compiladores. A lista `transformNames` percorre a árvore de tipos em ordem (para
tuplas aninhadas).

### 4. Layout e passagem no runtime

`ValueTuple<int,int>` é um struct comum com layout sequencial (ver `10`). O JIT trata campos de
structs promovíveis (replace de campos por registradores): o comentário em
`src/coreclr/vm/jitinterface.cpp` ~2148 usa exatamente `ValueTuple<int,int>[4]` para ilustrar
sub-campos/strides de um array de structs ao descrever a promoção/layout ao JIT. Retornar uma
tupla evita alocação (vs. `Tuple<>` ou `out` params).

Por ser genérico sobre value types, cada `ValueTuple<int,string>` **instancia** código próprio (ver
`03`), enquanto `ValueTuple<string,string>` compartilha código canônico (`__Canon`) quando os
argumentos são referências — mas, como é struct, o layout do *tuple* depende do tamanho dos
campos; com referências o layout é idêntico.

### 5. Deconstrução

Há duas formas, ambas **do compilador**:
1. Para tuplas: `var (a, b) = t;` é traduzido para acesso direto a `Item1`, `Item2` (sem chamada).
2. Para tipos arbitrários: o compilador procura um método `Deconstruct(out T1, out T2, ...)` de
   instância ou extensão. O CoreLib fornece, para as tuplas de classe,
   `TupleExtensions.Deconstruct<T1,...>(this Tuple<T1,...>, out T1, ...)`
   (`src/libraries/System.Private.CoreLib/src/System/TupleExtensions.cs` ~16-21), além de
   `ToValueTuple()`/`ToTuple()` para conversão entre as duas famílias.
Nada disso exige suporte do runtime além de ter genéricos e structs.

### 6. Interação com `ITuple`/boxing/ref structs

- Passar tupla como `ITuple`/`object` faz boxing (ver `06`).
- Argumentos `ref struct` em genéricos só são aceitos por parâmetros marcados `AcceptByRefLike`
  (`byreflike-generics.md`). Não verifiquei se `ValueTuple<...>` carrega esse flag; sem ele, uma
  tupla de `Span<T>` é rejeitada.
- A constraint `where TRest : struct` na 8ª aridade é o runtime/compilador garantindo que
  o "resto" seja uma `ValueTuple`.

## Por quê

- **Retornos múltiplos sem alocação**: `Tuple<>` aloca; `ValueTuple` é struct e fica na stack/
  registradores (motivação de desempenho; inferência pelo design struct + campos públicos).
- **Campos em vez de propriedades**: menor custo e permite `ref` aos campos (`ref t.Item1`) e
  mutação in-place (conhecimento geral sobre o design).
- **Nomes como metadado**: preservar a identidade estrutural do tipo — assim tuplas de assemblies
  diferentes são compatíveis e não é preciso novo tipo por nome.
- **Aninhamento no 8º elemento**: manter um conjunto finito de tipos no CoreLib, sem limite de
  aridade.
- **Deconstrução baseada em convenção**: qualquer tipo pode participar sem interface especial.

## Trade-offs

- **Mutáveis**: cópias podem divergir; mutação de `readonly` local gera cópias (armadilha clássica
  de structs mutáveis).
- **Nomes não são runtime**: reflexão não os vê sem ler atributos; `ToString()` mostra `(1, 2)` sem
  nomes; mudar nomes não é breaking binário, mas é semântico.
- **Aridade >7 aninhada**: `Rest`/`Item8` confuso na depuração, custa tamanho.
- **Custo de código genérico**: muitas instanciações geram código por combinação de value types.
- **Boxing** via `ITuple`.
- **Igualdade estrutural usa comparer padrão por elemento**: pode ser mais lento que um record
  especializado.
- **Duplicação de famílias** (`Tuple` vs `ValueTuple`) e conversões.

## Como aplicar na sua linguagem

1. **Tuplas anônimas como açúcar sobre um tipo genérico regular** (struct ou record) evita criar
   novo conceito no runtime; todas as ferramentas já entendem genéricos.
2. **Struct para retorno múltiplo, class só se quiser identidade/imutabilidade profunda**. Se
   oferecer só uma, prefira a de valor.
3. **Nomes como metadado** ("transform names") dá a ergonomia dos nomes sem multiplicar tipos;
   registre a ordem de percurso da árvore de tipos para tuplas aninhadas.
4. **Defina o limite de aridade**: ou ilimitado por aninhamento (como `TRest`) ou erro. Esconda o
   aninhamento no compilador.
5. **Decida a mutabilidade**: imutável por padrão (`readonly`) evita a classe de bugs de structs
   mutáveis; o .NET escolheu mutável por desempenho.
6. **Deconstrução por convenção** (`Deconstruct`) + regra especial rápida para tuplas literais.
7. **Ofereça uma interface de acesso dinâmico** (`ITuple`) separada, para reflexão/pattern matching,
   deixando claro que ela causa boxing.
8. **Igualdade e hash estruturais** gerados por tipo (ou por comparer padrão por elemento).

### Exemplo de tradução (esboço do que um compilador faz)

```
// código-fonte (sintaxe hipotética)
fn divmod(a: int, b: int) -> (q: int, r: int) { return (a / b, a % b) }
let (q, r) = divmod(7, 2)

// lowering sugerido (espelha o C#)
struct ValueTuple<T1, T2> { T1 Item1; T2 Item2 }          // tipo de biblioteca
[TupleElementNames("q","r")] ValueTuple<int,int> divmod(int a, int b)
var tmp = divmod(7, 2); int q = tmp.Item1; int r = tmp.Item2;
```

### Checklist de decisões

- [ ] Tupla é tipo de biblioteca ou tipo primitivo do runtime? (.NET: biblioteca.)
- [ ] Nomes participam da identidade do tipo? (.NET: não.)
- [ ] Mutável ou imutável? (.NET: mutável.)
- [ ] Como representar aridade grande? (.NET: `TRest` aninhado.)
- [ ] A deconstrução é extensível? (.NET: por convenção `Deconstruct`.)
- [ ] Existe acesso dinâmico por índice (`ITuple`) e qual seu custo (boxing)?

### Observação final

`ValueTuple` demonstra um princípio útil de design: uma feature de linguagem (tuplas com nomes e
deconstrução) pode ser entregue quase inteiramente por **tipos de biblioteca + atributos de
metadado + lowering do compilador**, sem nova categoria de tipo no runtime.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/ValueTuple.cs`
- `src/libraries/System.Private.CoreLib/src/System/Tuple.cs`, `TupleExtensions.cs`, `TupleSlim.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ITuple.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/TupleElementNamesAttribute.cs`
- `src/coreclr/vm/jitinterface.cpp` (~2148, exemplo `ValueTuple<int,int>[4]`)
- `docs/design/features/byreflike-generics.md`
