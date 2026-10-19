# Generic constraints (where)

## Mecanismo no .NET

### 1. Três famílias de constraint (metadado)

`docs/design/coreclr/botr/type-loader.md` (seção 2.2, "Generic Constraint") descreve três tipos:

1. **Constraints especiais**
   - *Reference type* — `class`
   - *Value type* — `struct`: "deve ser um value type diferente de `System.Nullable<T>`"
   - *Default constructor* — `new()`
2. **Base type constraints** — o argumento deve derivar de um tipo.
3. **Interface constraints** — o argumento deve implementar uma interface.

Os flags especiais ficam no metadado `GenericParam` (`CorGenericParamAttr`):
`gpReferenceTypeConstraint`, `gpNotNullableValueTypeConstraint`, `gpDefaultConstructorConstraint`,
e o novo `gpAllowByRefLike` (0x0020) descrito em `docs/design/features/byreflike-generics.md`.
Todos são lidos em `src/coreclr/vm/typedesc.cpp` e `siginfo.cpp` via
`flags & gpSpecialConstraintMask`.

A doc diz que a verificação é feita **pelo type loader**: "it is enforced by the type loader".

### 2. Onde as constraints vivem em runtime

`TypeVarTypeDesc` (`src/coreclr/vm/typedesc.h`, ~linha 299) representa a variável `T`. Ela carrega:
- o token de `GenericParam` (para ler os flags do metadado),
- constraints em cache (`GetCachedConstraints`), carregadas **em níveis** — o enum
  `WhichConstraintsToLoad` (ex.: `TypeOrMethodVarsAndNonInterfacesOnly`, `All`), com bits
  `WhichConstraintsLoadedMask/Shift` (`typedesc.h` ~301-302, 366). Carregar interfaces é mais
  caro e pode introduzir ciclos, então só se carrega quando necessário.
- `SatisfiesConstraints(SigTypeContext*, TypeHandle thArg, ...)` (`typedesc.h` ~378) — a
  verificação principal.

### 3. Regras de satisfação (de `typedesc.cpp`)

`SatisfiesSpecialConstraintRecursive` (~linhas 1260-1310):
- `gpNotNullableValueTypeConstraint`: o argumento satisfaz se ele mesmo tem o flag `struct`, ou é um
  value type concreto que não seja `Nullable<>` (ver ~linha 1358:
  `thAncestorOfType.IsValueType() && !IsNullable()`).
- `gpReferenceTypeConstraint`: "não é herdado" quando o argumento é uma variável que apenas *herda*
  de outra constrangida (comentário em `ConstrainedAsObjRefHelper`: `class A<S,T> where S:T where T:class`
  não garante que `S` seja ObjRef).
- `gpDefaultConstructorConstraint`: também "não herdado" por variável; é satisfeito quando o
  argumento tem `new()` ou é `struct` (todo struct tem construtor padrão implícito).

Além disso, o runtime implementa duas consultas auxiliares usadas por outras partes (casting, JIT):
- `ConstrainedAsObjRef()` — a variável é necessariamente um tipo de referência? É verdadeiro se tem
  `class`, ou se alguma constraint de classe que não seja `Object`, `ValueType` ou `Enum` é um
  tipo de referência (o código exclui explicitamente `g_pObjectClass`, `g_pValueTypeClass`,
  `g_pEnumClass`).
- `ConstrainedAsValueType()` — verdadeiro se tem `struct` ou alguma constraint não-objref.

Outra regra do sistema de tipos: uma variável genérica "boxed" pode ser convertida para suas
constraints, ou para `object`, e para `ValueType` se tiver `struct` (`TypeDesc::CanCastTo`,
`typedesc.cpp` ~300-345).

### 4. Compatibilidade de constraints entre declarações

`siginfo.cpp` (~4984-5030) compara listas de constraints de dois parâmetros genéricos (para
`MethodImpl`/overrides e verificar equivalência de assinaturas): um parâmetro mais constrangido
no lado A exige o mesmo (ou mais forte) no lado B; `AllowByRefLike` vai em sentido inverso (o
override pode *aceitar mais*, não menos). Mostra que constraints participam da *subtipagem de
assinaturas*.

### 5. Constraints mudam a semântica da execução

- `constrained. callvirt`: o JIT pode chamar um método de interface/virtual em `T` sem boxing se
  `T` for value type que o implementa (`Ecma-335-Augments.md` ~linha 434 menciona que com default
  interface methods o boxing implícito torna-se observável).
- Tipos ByRefLike como argumento: `byreflike-generics.md` define que `constrained. callvirt` é
  válido, mas lança `NotSupportedException` se o alvo for método de `object` ou DIM; instruções
  `box`, `newarr`, `stsfld` etc. falham com `InvalidProgramException`/`TypeLoadException`.
- **Static virtual members / abstrações numéricas**: `docs/design/specs/Ecma-335-Augments.md`
  (~linhas 500-760) define membros estáticos virtuais em interfaces, resolvidos pelo tipo em
  tempo de execução. Isso é o que viabiliza constraints como `where T : INumber<T>`; por exemplo,
  `Enum.ToString<TUnderlying,TStorage>` em
  `src/libraries/System.Private.CoreLib/src/System/Enum.cs` (~1482) usa
  `where TUnderlying : struct, INumber<TUnderlying>, IBitwiseOperators<...>, IMinMaxValue<...>`.

### 6. `Nullable<T>` e a constraint `struct`

`Nullable<T>` é declarado `where T : struct`
(`src/libraries/System.Private.CoreLib/src/System/Nullable.cs`, linha 21). Como a constraint
`struct` exclui `Nullable<>`, `Nullable<Nullable<int>>` é inválido. Ver `09`.

## Por quê

- **Habilitar operações dentro do corpo genérico** sem conhecer o argumento: chamar métodos da
  interface/base, usar `new T()`, tratar como value type ou ref type.
- **Verificar no carregamento** em vez de por-uso: a checagem acontece quando o tipo é instanciado,
  não a cada chamada.
- **Informar otimizações**: `ConstrainedAsObjRef`/`ConstrainedAsValueType` dizem ao JIT se `T` é
  ref/valor mesmo em código genérico não instanciado.
- **`AllowByRefLike`** é "opt-in" porque os genéricos antigos assumem que `T` pode ser boxed ou ser
  campo de classe/array; sem o flag, código existente quebraria.

## Trade-offs

- A linguagem de constraints é **limitada e fixa** (não há `where T : not struct`, uniões,
  constraints por operador — operadores só via static virtual em interfaces). (Observação geral; o
  conjunto lido acima é exatamente o que o runtime codifica.)
- Constraints sobre variáveis têm regras sutis de herança (`class` e `new()` *não* se propagam
  através de uma variável).
- Interface constraints obrigam carregamento adicional e cuidados com ciclos (níveis de load).
- Cada nova constraint (`AllowByRefLike`) toca em metadado, compiladores, trimmer e outras
  linguagens — custo de evolução alto (lista no `byreflike-generics.md`).
- Constraints de interface com value type: chamadas via `constrained.` evitam boxing, mas
  só se o JIT reificar o tipo (não em código compartilhado de referência).

## Como aplicar na sua linguagem

1. **Modele constraints como parte da assinatura do parâmetro**: um conjunto de predicados
   especiais (é-ref, é-valor-não-nulo, tem-ctor-padrão) + lista de supertipos.
2. **Decida o momento da verificação**: no .NET é na instanciação (carga). Se sua linguagem
   instancia por monomorfização, verifique também no site de definição.
3. **Defina claramente quais propriedades se propagam por variáveis**. A lição do código:
   `class`/`new()` não propagam; supertipo propaga. Escreva isso na spec.
4. **Separe constraints de "capacidades" (traits/interfaces com membros estáticos)** de constraints
   de "categoria" (ref/valor). A segunda categoria afeta representação; a primeira, só operações.
5. **Considere constraints negativas ou opt-in** (como `AllowByRefLike`) para evoluir sem quebrar
   código existente: o padrão é restritivo e o autor relaxa explicitamente.
6. **Se tiver generics numéricos**, planeje membros estáticos abstratos em interfaces desde o
   início — foi o que permitiu `INumber<T>` no .NET.
7. **Verifique compatibilidade de constraints em overrides** (regra siginfo.cpp: override não
   pode exigir mais do que a base).

### Exemplo: propagação de constraints por variáveis

```csharp
class A<S, T> where S : T where T : class { }
// S é obrigatoriamente subtipo de T, mas S NÃO é garantidamente tipo de referência
// (ConstrainedAsObjRefHelper: gpReferenceTypeConstraint "não propaga para cima").
```

Já uma constraint de classe concreta propaga: `where T : Stream` torna `T` necessariamente de
referência (`ConstrainedAsObjRef` devolve TRUE se alguma constraint não-interface, exceto
`Object`/`ValueType`/`Enum`, é referência).

### Checklist de decisões

- [ ] Que predicados "de categoria" existem (ref, valor, ctor padrão, unmanaged, allows-ref-struct)?
- [ ] Como são herdados/propagados por variáveis de tipo?
- [ ] Verificação na definição, na instanciação, ou nas duas?
- [ ] Como constraints entram na comparação de assinaturas de override?
- [ ] Há membros estáticos abstratos em interfaces para operadores?

## Referências

- `docs/design/coreclr/botr/type-loader.md` (seção 2.2, constraints)
- `docs/design/features/byreflike-generics.md`
- `docs/design/specs/Ecma-335-Augments.md` (static virtual members, constrained.)
- `src/coreclr/vm/typedesc.h` (`TypeVarTypeDesc`, `WhichConstraintsToLoad`)
- `src/coreclr/vm/typedesc.cpp` (`SatisfiesConstraints`, `ConstrainedAsObjRef`, `ConstrainedAsValueType`, `SatisfiesSpecialConstraintRecursive`)
- `src/coreclr/vm/siginfo.cpp` (~4984-5030, comparação de constraints)
- `src/libraries/System.Private.CoreLib/src/System/Nullable.cs`
- `src/libraries/System.Private.CoreLib/src/System/Enum.cs`
