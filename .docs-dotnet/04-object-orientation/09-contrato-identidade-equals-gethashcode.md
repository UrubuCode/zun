# Contrato de identidade: Equals / GetHashCode / ReferenceEquals

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### Três noções de "igual"

1. **Identidade de referência**: mesmo objeto no heap. `object.ReferenceEquals(a, b)` e o operador `==` sobre `object`.
2. **Igualdade de valor definida pelo tipo**: `virtual bool Equals(object?)` (e, por convenção, `IEquatable<T>.Equals(T)`).
3. **Hash compatível**: `virtual int GetHashCode()`.

O contrato (C#/ECMA, documentação pública): `Equals` é reflexivo, simétrico e transitivo; objetos iguais devem ter o mesmo hash; o hash de um objeto não deve mudar enquanto ele estiver numa coleção hash.

### A implementação padrão em System.Object (repo)

`src/libraries/System.Private.CoreLib/src/System/Object.cs`:

```csharp
public virtual bool Equals(object? obj) { return this == obj; }

public static bool Equals(object? objA, object? objB) {
    if (objA == objB) return true;
    if (objA == null || objB == null) return false;
    return objA.Equals(objB);
}

[NonVersionable]
public static bool ReferenceEquals(object? objA, object? objB) { return objA == objB; }

public virtual int GetHashCode() { return RuntimeHelpers.GetHashCode(this); }
```

Comentário no próprio arquivo sobre `GetHashCode`: o padrão "technically meet the needs of a hash function, but it's less than ideal. Objects (& especially value classes) should override this method." Ou seja, para classes sem override: igualdade = identidade e hash = hash de identidade. `ReferenceEquals` é *estático e não virtual*, o que garante que ninguém consegue sobrescrever a noção de identidade (decisão confirmada pelo código: não existe `virtual`).

### De onde vem o hash de identidade (repo)

`RuntimeHelpers.GetHashCode(object)` (`src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs` ~252) chama `TryGetHashCode` (intrínseco rápido) e, se o objeto ainda não tem hash, o caminho lento nativo `ObjectNative_GetHashCodeSlow`. No nativo, `Object::GetHashCodeEx()` (`src/coreclr/vm/object.cpp` ~77) mostra o mecanismo:

- O hash fica no *header* do objeto (a palavra antes do ponteiro de `MethodTable`: `OBJHEADER_SIZE` em `object.h` ~84), compartilhando espaço com o "thin lock" e o índice de sync block. Bits relevantes: `BIT_SBLK_IS_HASH_OR_SYNCBLKINDEX`, `BIT_SBLK_IS_HASHCODE`, `MASK_HASHCODE`.
- Caso comum: o header já contém o hash -> devolve `bits & MASK_HASHCODE`.
- Se o header tem índice de sync block, o hash fica no `SyncBlock` (`psb->GetHashCode()` / `SetHashCode`).
- Senão, gera um novo (`ComputeHashCode`, `object.cpp` ~36): usa o gerador congruencial linear da thread (`Thread::GetNewHashCode`) ou um global (`GetGlobalNewHashCode`, "linear congruential generator from Knuth Vol. 2"), pega os bits altos ("more random"), força `!= 0`, e o armazena de forma atômica com laço de retry (há spin com `YieldProcessorNormalized` se o bit de spin-lock estiver ligado).
- O hash é *preguiçoso*: só é alocado/gravado quando alguém o pede. O hash *não* é o endereço do objeto, porque o GC move objetos (compactação); por isso é guardado no objeto (inferência bem sustentada: o código grava o valor no header em vez de derivar do endereço, e o comentário em `ComputeHashCode` menciona o caso de objeto "being frozen").
- `Object::TryGetHashCode` (`object.inl` ~63) devolve 0 se ainda não há hash ("not set").

### Value types: igualdade estrutural (repo)

`src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`:

- `Equals(object?)`: false se `null` ou tipos diferentes; se `CanCompareBitsOrUseFastGetHashCode(MethodTable*)` é verdadeiro, compara bytes crus (`SpanHelpers.SequenceEqual` sobre `GetNumInstanceFieldBytes()`); senão, usa *reflexão* sobre os campos de instância e chama `Equals` em cada um.
- `CanCompareBitsOrUseFastGetHashCode` consulta flags no MethodTable (`enum_flag_HasCheckedCanCompareBitsOrUseFastGetHashCode`, `enum_flag_CanCompareBitsOrUseFastGetHashCode`, `methodtable.h` ~1161): vale quando o struct não tem referências GC, não tem campos de ponto flutuante com semântica especial e nenhum campo (recursivamente) sobrescreve `Equals`/`GetHashCode`. O resultado é cacheado no `MethodTableAuxiliaryData`.
- `GetHashCode()` mistura o ponteiro do `MethodTable` (para reduzir colisões entre tipos) e ou hasheia os bytes (`HashCode.AddBytes`) ou, no caso lento, escolhe uma estratégia (`GetHashCodeStrategy`) para usar o hash de um campo.
- Comentário no código: a implementação padrão "reveals the value of the fields", então structs com dados sensíveis devem sobrescrever.

Ponto-chave: a igualdade estrutural padrão para structs existe, mas é cara no caso geral e propensa a surpresas (o repo documenta a necessidade da checagem de "bits comparáveis" justamente porque `float` e padding invalidam o `memcmp`).

### Outros peças relacionadas

- `IEquatable<T>` (`System/IEquatable.cs`, repo): `bool Equals(T? other)`. Comentário no código: "invariant due to questionable semantics around equality and inheritance" -> o parâmetro de tipo é invariante de propósito.
- `EqualityComparer<T>.Default` (`System/Collections/Generic/EqualityComparer.cs`; o `Default` é "runtime-specific", repo) escolhe o caminho `IEquatable<T>` sem boxing para evitar `Equals(object)`.
- `String` define igualdade por conteúdo (`System/String.Comparison.cs`, `EqualsHelper`), com `ReferenceEquals` como atalho rápido (~126).
- Coleções hash (`Dictionary`, `HashSet`) chamam `GetHashCode` + `Equals`.

## Por quê

1. **Uma raiz fornece contrato universal.** Todo objeto pode ser chave de hash, comparado, impresso (arquivo 10).
2. **Padrão seguro e barato**: classes por padrão têm semântica de identidade (O(1), sem custo) e podem optar por valor.
3. **`ReferenceEquals` não virtual** garante um "escape" para identidade (ex.: checar `null`, caches, grafos).
4. **Value types com igualdade por valor** porque valores não têm identidade distinguível; um struct default comparado por referência não faria sentido.
5. **Hash no header e lazy**: não paga memória/tempo para objetos nunca usados como chave; o header já existe por causa de lock/sync block (o mesmo espaço serve a vários usos).
6. **Hash independente do endereço**, por causa do GC compactador.

## Trade-offs

- **Contrato não imposto pelo compilador** (C# só dá warning CS0659/CS0661 se se sobrescreve um sem o outro): bugs comuns (hash inconsistente com Equals, hash mutável).
- **`Equals(object)` implica boxing e cast** para tipos de valor; mitigado por `IEquatable<T>` e comparers genéricos.
- **Igualdade padrão de struct pode usar reflexão** (caminho lento visto no `ValueType.Equals`) e é frágil com float/NaN/padding.
- **Hash de identidade de 26-30 bits e não uniforme?** O `ComputeHashCode` ajusta a `HASHCODE_BITS` (menor que 32) para caber no header: colisões possíveis; hash não é garantidamente único (C#/ECMA).
- **Herança e simetria**: `a.Equals(b)` vs `b.Equals(a)` com tipos derivados pode quebrar; `IEquatable<T>` invariante é uma reação a isso (comentário no repo).
- **Mutabilidade** invalida chaves em dicionários.
- **Cache `CanCompareBits...`** e reflexão aumentam complexidade do runtime.

## Como aplicar na sua linguagem

1. **Separe três operações com nomes diferentes**: identidade (`same(a,b)`, não sobrescrevível), igualdade (`==`/`equals`) e hash. Não faça `==` ambíguo entre identidade e valor.
2. **Padrão para tipos referência**: identidade. Padrão para tipos valor: estrutural, gerado pelo compilador (campo a campo), não por reflexão, e sem a armadilha de memcmp com floats.
3. **Acople `equals` e `hash`**: gere/exija os dois juntos (ou derive um do outro). Considere `derive(Eq, Hash)` estilo Rust ou `record` do C#, em vez do par manual.
4. **Torne a igualdade tipada** (`equals(other: Self)`) e o `Equals(object)` um adaptador, para evitar boxing. O .NET chegou lá com `IEquatable<T>` depois do fato.
5. **Hash de identidade**: não use o endereço se o GC move objetos. Guarde num campo do header preguiçosamente (modelo do `Object::GetHashCodeEx`) ou use um contador monotônico.
6. **Decida a semântica de NaN** (`NaN != NaN` vs hash/igualdade reflexiva). O .NET separa `==` (IEEE) de `Equals` (reflexivo) em `Double` (C#/ECMA, não verificado aqui).
7. **Defina a política de mutabilidade de chaves**: proíba, avise, ou use tipos imutáveis para chaves.
8. **Especifique o contrato em testes de propriedade** (reflexividade, simetria, transitividade, consistência hash).
9. **Defina igualdade com herança**: igualdade exata de tipo (`GetType() == other.GetType()`, como `ValueType.Equals`) ou por subtipo; documente.
10. **Espaço de header**: se quiser hash/locks lazily no objeto, reserve uma palavra de header com bits multiplexados (como `ObjHeader`) e um mecanismo de "promoção" para estrutura maior quando necessário (sync block).

Pseudocódigo do hash de identidade:

```
identity_hash(obj):
    h = obj.header.hash_bits
    if h != 0: return h
    h = next_random_nonzero(HASH_BITS)
    CAS(obj.header.hash_bits, 0, h)   // ou promover p/ estrutura grande
    return obj.header.hash_bits
```

## Resumo dos fatos confirmados

- `Object.Equals` padrão é identidade (`this == obj`); `Object.GetHashCode` usa `RuntimeHelpers.GetHashCode`.
- `Object.ReferenceEquals` é estático, `[NonVersionable]`, não sobrescrevível.
- Hash de identidade vive no header do objeto ou no `SyncBlock`; é gerado preguiçosamente com gerador congruencial linear (por thread ou global) e nunca é 0.
- `Object::TryGetHashCode` devolve 0 se ainda não existe hash.
- `ValueType.Equals` compara bytes se `CanCompareBitsOrUseFastGetHashCode`, senão usa reflexão por campo.
- `ValueType.GetHashCode` mistura o ponteiro de `MethodTable` e os bytes dos campos.
- `IEquatable<T>` é invariante propositalmente (comentário no arquivo).
- `String` compara por conteúdo e usa `ReferenceEquals` como atalho.

## Regras do contrato (C#/ECMA, documentação pública)

1. `x.Equals(x)` é verdadeiro.
2. `x.Equals(y)` == `y.Equals(x)`.
3. Se `x.Equals(y)` e `y.Equals(z)`, então `x.Equals(z)`.
4. Chamadas repetidas devolvem o mesmo resultado se nenhum estado envolvido mudou.
5. `x.Equals(null)` é falso.
6. `x.Equals(y)` implica `x.GetHashCode() == y.GetHashCode()`.
7. O hash não deve mudar enquanto o objeto é chave de uma coleção hash.
8. `GetHashCode` não deve lançar exceção.

## Armadilhas

- Sobrescrever `Equals` sem `GetHashCode` quebra dicionários.
- Incluir campos mutáveis no hash.
- Igualdade com herança: `Point` vs `ColoredPoint` gera assimetria se usar `is` em vez de tipo exato.
- Ponto flutuante: `NaN != NaN` em `==`, mas `Equals` deve ser reflexivo (C#/ECMA).
- Struct sem override: pode cair no caminho de reflexão (lento).
- Usar o hash de identidade como valor persistente: ele varia entre execuções.
- Assumir que o hash de identidade é único: colisões são possíveis.

## Perguntas de design

- `==` em tipos referência compara identidade ou valor por padrão?
- A igualdade estrutural é gerada pelo compilador (records/derive)?
- Há um tipo de chave (`Hashable`) obrigatório em dicionários?
- Hash de identidade estável durante a vida do objeto, mesmo com GC que move?
- A linguagem define semântica para NaN, `-0.0` e `+0.0` em igualdade/hash?
- Qual a política de igualdade entre tipos diferentes (`1 == 1L`)?

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Object.cs`: `Equals`, `ReferenceEquals`, `GetHashCode`.
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs` (~246-268): `TryGetHashCode`, `GetHashCodeSlow`, `GetHashCode(object)`.
- `src/coreclr/vm/object.cpp` (`ComputeHashCode` ~36, `GetHashCodeEx` ~77, `GetGlobalNewHashCode`), `src/coreclr/vm/object.inl` (`TryGetHashCode` ~63), `src/coreclr/vm/object.h` (`OBJHEADER_SIZE`, `GetSyncBlock`).
- `src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`: `Equals`, `GetHashCode`, `CanCompareBitsOrUseFastGetHashCode`.
- `src/coreclr/vm/methodtable.h` (~1161): flags `HasCheckedCanCompareBitsOrUseFastGetHashCode`.
- `src/libraries/System.Private.CoreLib/src/System/IEquatable.cs`, `.../Collections/Generic/EqualityComparer.cs`, `.../String.Comparison.cs`.
