# Static abstract members em interfaces (generic math)

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### A ideia

Uma interface normal descreve o que *instâncias* sabem fazer. Static abstract/virtual members descrevem o que o *tipo* sabe fazer: construtores de valor (`Zero`, `One`), operadores (`+`), parsers (`Parse`), etc. Combinados com genéricos restritos (`where T : INumber<T>`), permitem escrever `T Soma<T>(T a, T b) => a + b` uma só vez e despachar, em tempo de especialização, para a implementação do tipo concreto.

Exemplos reais no repo (CoreLib):

- `src/libraries/System.Private.CoreLib/src/System/Numerics/IAdditionOperators.cs`:
  ```csharp
  public interface IAdditionOperators<TSelf, TOther, TResult>
      where TSelf : IAdditionOperators<TSelf, TOther, TResult>?
  {
      static abstract TResult operator +(TSelf left, TOther right);
      ...
  }
  ```
- `IAdditiveIdentity.cs`: `static abstract TResult AdditiveIdentity { get; }`.
- `INumberBase.cs`: `static abstract TSelf One { get; }`, `Radix`, `Zero`, `Abs(TSelf)`, `IsCanonical(TSelf)`, etc. `INumber<TSelf>`, `IBinaryInteger<TSelf>` e outras compõem esses blocos. `Int32` implementa a pilha (`src/libraries/System.Private.CoreLib/src/System/Int32.cs`).

Note o padrão CRTP/"self type": `where TSelf : IAdditionOperators<TSelf, ...>`. O parâmetro `TSelf` carrega o tipo implementador para que assinaturas estáticas possam falar do próprio tipo.

### Mudanças de runtime/IL (repo)

Documentadas em `docs/design/specs/Ecma-335-Augments.md`, seção "Static Interface Methods":

- Métodos estáticos de interface podem ser `virtual`; tipos implementadores fornecem implementação por MethodImpl (`.override`). "Polymorphic behavior of calls to these methods is facilitated by the `constrained.` call IL instruction where the `constrained.` prefix specifies the type to use for lookup of the static interface method." (I.8.4.4).
- `constrained.` agora é permitido em `call` e `ldftn` (além de `callvirt`). Tabela de transição de pilha: `constrained. implementorType call method`. "The behavior of the `constrained.` prefix is to change the method that the call or ldftn instruction refers to be the method on implementorType which implements the virtual static method." (III.2.1).
- Em II.15.2: "For static virtual methods, the particular method to call is determined via a type lookup based on the `constrained.` IL instruction prefix or on generic type constraints but the call itself doesn't involve any instance or `this` pointer."
- Em II.9.11, argumentos genéricos devem satisfazer *implements-all-static-interface-methods-of* cada constraint.
- Em II.10.3.2, um método pode implementar um virtual estático via `.override` por um "non-virtual static method" com mesma assinatura.

### No type loader (repo)

`src/coreclr/vm/methodtablebuilder.cpp`:

- Na enumeração de métodos: `bool isStaticVirtual = (IsMdVirtual(dwMemberAttrs) && IsMdStatic(dwMemberAttrs)); if (isStaticVirtual) bmtProp.fHasVirtualStaticMethods = TRUE;` (~linha 2895). Há flag por tipo indicando que existem virtuais estáticos.
- Em `TestMethodImpl` (~4870-4905): "Virtual methods on classes/valuetypes cannot be static" (`IDS_CLASSLOAD_STATICVIRTUAL` se o decl é estático e o dono não é interface); impl e decl precisam ter o mesmo status de estático; um impl estático precisa ser *não virtual* ou virtual-não-abstrato (`IDS_CLASSLOAD_MI_MUSTBEVIRTUAL` na combinação ruim).
- Na vtable: `if (!IsMdVirtual(it.Attrs()) || IsMdStatic(it.Attrs()))` (~5380) exclui estáticos da atribuição de slots de vtable de instância. Ou seja, virtuais estáticos *não ocupam slots de vtable* e não usam VSD.
- Também `~5117: if (IsMdStatic(it.Attrs()) && IsMdVirtual(it.Attrs()))` trata esses métodos à parte na construção de interface map/implementações (não li a lógica completa).

Em tempo de execução, a resolução é do tipo "dado `T` concreto e `IFoo.StaticMember`, ache a implementação em `T`". Isso é feito no JIT/ao resolver tokens (via o contrato JIT-EE; detalhe de implementação em `src/coreclr/vm/jitinterface.cpp`, que não li) (inferência).

### Interação com genéricos e especialização

- Para `T` = struct, o runtime gera código especializado por instanciação, de modo que `T.Zero` e `a + b` resolvem para chamadas diretas e inlináveis (inferência baseada no modelo de instanciação de structs; ver `docs/design/coreclr/botr/shared-generics.md`, que descreve o compartilhamento de código para tipos referência apenas, repo).
- Para `T` = referência, o código é compartilhado (canônico `__Canon`), e a resolução do estático virtual precisa de um dicionário genérico/lookup em runtime (inferência; consistente com `shared-generics.md`).

### Limites

- Não se pode chamar um membro estático abstrato diretamente por um tipo interface (`IFoo.Bar()` fora de contexto genérico) (C#; regra da linguagem).
- Interfaces com estáticos abstratos não são usáveis como tipo de variável comum para chamada de instância "sem genéricos" em todos os cenários (C#).
- `static virtual` (com corpo padrão) em interface também existe no CLI (virtual + corpo), mas as especificações de augments focam nos casos em geral.

## Por quê

1. **Matemática genérica sem boxing nem delegates.** Antes, para somar `T` genérico era preciso passar um `Func<T,T,T>`/comparer/calculadora (`Calculator<T>`). Interface de instância implicaria criar instância de `T`, boxing ou parâmetros extras.
2. **Abstrair construtores e parsers.** `Zero`, `One`, `Parse` não têm instância. Static abstract permite expressá-los num contrato.
3. **Custo zero em structs** via especialização e `constrained. call` (inferência por design).
4. **Reuso do sistema de constraints** existente: a mesma sintaxe `where T : I` serve.
5. O `INumber<T>` etc. no repo mostram que o objetivo era unificar tipos numéricos primitivos (`Int32`, `Double`...) sob hierarquias de contratos pequenos e composáveis (`IAdditionOperators`, `IAdditiveIdentity`, ...).

## Trade-offs

- **Complexidade de tipos**: assinaturas como `where TSelf : IAdditionOperators<TSelf, TOther, TResult>?` são verbosas; a hierarquia numérica tem dezenas de interfaces.
- **Pilha de constraints recursivas (CRTP)**: mensagens de erro difíceis; inferência de tipo limitada.
- **Não são polimorfismo dinâmico**: você não pode guardar "qualquer coisa que seja INumber" numa lista heterogênea e chamar `+`: o tipo precisa ser conhecido em tempo de especialização.
- **Custo no runtime**: novo conceito de `.override` estático, regras no loader, mais código em JIT, reflexão e AOT (trimmers precisam preservar implementações).
- **Versionamento**: adicionar um static abstract a uma interface publicada quebra todos os implementadores (a menos que haja implementação padrão, `static virtual`).
- **Código compartilhado para referências**: dicionários genéricos têm overhead em relação à especialização de structs.

## Como aplicar na sua linguagem

1. **Se você tem genéricos e tipos numéricos como valores, vale a pena.** O ganho é eliminar o `Calculator<T>`/traits manuais. Alternativa: *type classes / traits* (Haskell, Rust, Swift protocols com requisitos estáticos), que já são o modelo natural; o design do .NET é uma forma de reconstruir type classes sobre interfaces.
2. **Defina que "membro de tipo" é diferente de "membro de instância"** no sistema de interfaces. Marque cada requisito como `instance` ou `static` na declaração do contrato.
3. **Dê um nome ao tipo implementador** (`Self`/`TSelf`). Em vez de CRTP verboso, ofereça `Self` embutido: o .NET exige `TSelf` explícito porque não tem essa noção no sistema de tipos.
4. **Resolução em tempo de especialização, não em tempo de execução.** Mantenha a chamada `T.f()` como "chamada direta ao implementador de `T`" depois da monomorfização. O equivalente do .NET é `constrained. T call IFoo::f`: um IL que carrega o tipo implementador no próprio opcode.
5. **Faça virtuais estáticos não ocuparem vtable**: eles são resolvidos por tipo (não por objeto). Use uma tabela por (tipo, requisito) ou resolva por monomorfização. O `MethodTableBuilder` exclui estáticos dos slots de instância.
6. **Valide conformidade na declaração do tipo e na instanciação** (equivalente a "implements-all-static-interface-methods-of").
7. **Planeje as interfaces numéricas pequenas e compostas** (aditivo, multiplicativo, identidade, comparação, parse, format) em vez de uma interface `Number` monolítica: é o padrão do repo.
8. **Considere defaults** (`static virtual`) para evolução de contratos sem quebrar implementadores.
9. **AOT/trimming**: se sua linguagem compila ahead-of-time, garanta que a análise de alcançabilidade enxergue implementações que só são usadas via constraint.

Esboço do lowering para uma linguagem monomorfizante:

```
fn sum<T: Add>(a: T, b: T) -> T { a + b }
sum::<i32>   =>  fn sum_i32(a: i32, b: i32) -> i32 { i32::add(a, b) }
```

## Resumo dos fatos confirmados

- IL: métodos estáticos de interface podem ser virtuais; implementações via `.override` (MethodImpl).
- `constrained.` agora vale também para `call` e `ldftn`, com o tipo implementador no prefixo.
- Constraints de genéricos exigem "implements-all-static-interface-methods-of".
- O loader marca tipos com virtuais estáticos (`fHasVirtualStaticMethods`).
- Virtuais estáticos são excluídos da atribuição de slots de vtable de instância.
- `TestMethodImpl` impõe: virtual estático só em interface; decl e impl devem concordar quanto a estático.
- CoreLib define a hierarquia numérica: `IAdditionOperators`, `IAdditiveIdentity`, `INumberBase`, `INumber`, `IBinaryInteger` etc.; `Int32` as implementa.

## Esboço de como uma chamada `T.Zero` é resolvida

1. O IL gerado para `T.Zero` em método genérico usa `constrained. !!T call` sobre o getter estático da interface.
2. Na instanciação `T = int`, o JIT resolve o método estático virtual para a implementação em `Int32`.
3. O resultado é uma chamada direta, normalmente inlinada, sem objeto, sem boxing.
4. Para `T` de referência (código compartilhado), a resolução precisa de informação extra (dicionário genérico) em runtime.

(Passos 1-2 seguem as Augments do ECMA; os passos 3-4 são inferências baseadas no modelo de genéricos do BOTR, `shared-generics.md`.)

## Comparação com outras linguagens (conhecimento geral)

- Rust: traits com funções associadas (`T::zero()` via trait).
- Haskell: type classes (`class Num a where fromInteger :: Integer -> a`).
- Swift: protocols com requisitos `static`.
- C++: concepts/templates com resolução por duck typing em tempo de instanciação.
- Java: sem equivalente nativo; usa objetos de estratégia.

## Perguntas de design

- Os requisitos estáticos exigem um nome para o tipo implementador (`Self`)?
- Pode-se declarar requisito estático com corpo padrão? (o CLI permite `virtual`)
- Como se trata o tipo da interface quando usada como valor (sem genéricos)? O .NET restringe.
- Resolução em tempo de compilação, de link, ou em runtime?
- Trimming/AOT: como o linker sabe quais implementações ficam vivas?
- Operadores definidos pelo usuário são requisitos estáticos? (em C# são `static abstract` com `operator`, ver `IAdditionOperators`)

## Glossário

- Static abstract: membro estático declarado numa interface, sem corpo, a ser implementado pelo tipo.
- Static virtual: o mesmo, com corpo padrão opcional.
- `constrained.`: prefixo de IL que fixa o tipo usado na resolução da chamada.
- Tipo implementador (`implementorType`): o tipo concreto cuja implementação é buscada.
- `TSelf`: parâmetro de tipo convencional que representa o tipo implementador (CRTP).
- Generic math: conjunto de interfaces numéricas em `System.Numerics` baseado nesses membros.
- MethodImpl: entrada de metadado que associa uma declaração a um corpo, incluindo estáticos.
- Código compartilhado: um corpo de código para várias instanciações de tipos de referência.
- Dicionário genérico: estrutura de runtime que fornece handles de tipo ao código compartilhado.

## Protótipo mínimo (pseudocódigo, proposta minha)

```
trait Zero { static fn zero() -> Self }
impl Zero for i32 { static fn zero() -> i32 { 0 } }
fn soma<T: Zero + Add>(xs: [T]) -> T {
    let acc = T::zero()
    for x in xs { acc = acc + x }
    return acc
}
```

## Referências

- `docs/design/specs/Ecma-335-Augments.md`: seção "Static Interface Methods" (I.8.4.4, II.9.9, II.9.11, II.10.3, II.10.3.2, II.12, II.15.2, III.2.1 `constrained.`).
- `src/coreclr/vm/methodtablebuilder.cpp`: `fHasVirtualStaticMethods` (~2895), `TestMethodImpl` (~4870-4905), exclusão de estáticos em slots (~5380).
- `src/libraries/System.Private.CoreLib/src/System/Numerics/` (`IAdditionOperators.cs`, `IAdditiveIdentity.cs`, `INumberBase.cs`, `INumber.cs`, `IBinaryInteger.cs`).
- `src/libraries/System.Private.CoreLib/src/System/Int32.cs`: implementa as interfaces numéricas.
- `docs/design/coreclr/botr/shared-generics.md`.
