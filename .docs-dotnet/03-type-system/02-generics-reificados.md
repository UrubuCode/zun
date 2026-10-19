# Generics reificados — por que o .NET reifica (vs erasure da JVM)

> Convenção: caminhos citados foram lidos no repositório. A comparação com a JVM usa conhecimento
> geral (não está no repositório) e está marcada como tal.

## Mecanismo no .NET

### 1. Reificação = o tipo instanciado existe em tempo de execução

No CoreCLR, `List<int>`, `List<string>` e `List<object>` são três tipos distintos, cada um com seu
próprio `MethodTable`. O carregador mantém uma tabela hash (por módulo carregador) para garantir que
"nunca haverá dois ou mais `MethodTable` representando o mesmo tipo" instanciado
(`docs/design/coreclr/botr/type-loader.md`, seção 2.2.2 "Sharing"). O argumento de tipo faz parte
da identidade: o `TypeHandle` é igual se e somente se representa o mesmo tipo.

Terminologia do carregador (`type-loader.md`, seção 2.2):
- *Generic parameter*: o `T` da declaração.
- *Generic argument*: o `int` em `List<int>`.
- *Instantiation*: a lista de argumentos; tipos "fechados" têm todos os argumentos concretos.
- Variáveis genéricas (`T`) são representadas por `TypeVarTypeDesc` (um `TypeDesc`), "nunca
  compartilhadas entre tipos ou métodos" (cada variável tem um único dono) — `type-loader.md` e
  `src/coreclr/vm/typedesc.h` (~linha 299).

### 2. O que a reificação permite (confirmado no runtime)

- **Tipos de valor sem boxing**: `List<int>` guarda `int[]` real; o código gerado é especializado.
  Instanciações apenas com value types "não são compartilhadas, e cada tipo instanciado ganha seu
  próprio `EEClass` não compartilhado" (`type-loader.md`, 2.2.2).
- **Checagens de tipo em runtime** usam o argumento: `CanCastByVarianceToInterfaceOrDelegate`
  (`src/coreclr/vm/methodtable.cpp`, ~linha 1207) compara `TypeDefRid`, módulo e cada argumento
  de tipo do `Instantiation`. Isso só é possível porque os argumentos existem no objeto
  (`GetInstantiation()`).
- **Reflexão**: o `System.Type` reflete os argumentos (`typeof(List<int>) != typeof(List<string>)`)
  (conhecimento geral; o `RuntimeType` é construído a partir do `TypeHandle`).
- **Restrições verificadas pelo carregador**: "enforced by the type loader" (`type-loader.md`,
  seção de constraints). Ver `04`.

### 3. Como o runtime evita explosão de código: canonicalização

Reificar *tipos* não obriga a duplicar *código*. O CoreCLR (e o ILCompiler/NativeAOT) usa
**forma canônica**:
- Todas as instanciações sobre tipos de referência compartilham um único corpo de código,
  compilado para `Method<__Canon>`. `__Canon` é um tipo interno
  (`src/coreclr/System.Private.CoreLib/src/System/__Canon.cs`: "Internal methodtable used to
  instantiate the 'canonical' methodtable for generic instantiations").
- Instanciações sobre value types recebem código próprio.
- O que difere entre instanciações compartilhadas (p.ex. o `TypeHandle` de `List<T>`) vem de uma
  **generic dictionary** por instanciação, consultada pelo código compartilhado
  (`docs/design/coreclr/botr/shared-generics.md`). Detalhes em `03`.

O mesmo conceito existe no compilador gerenciado de NativeAOT: `CanonicalFormKind` e
`IsCanonicalSubtype` em `src/coreclr/tools/Common/TypeSystem/Canon/CanonTypes.cs`
(com `__Canon` e `__UniversalCanon`).

### 4. Onde isso aparece no IL / metadados

Genéricos são parte do formato de metadado e do IL (ECMA-335): `GenericParam` com flags de
variância e constraints, e instruções que referenciam o contexto genérico. O arquivo
`docs/design/features/byreflike-generics.md` mostra como o runtime estende isso (novo flag
`gpAcceptByRefLike = 0x0020`, tratado em `src/coreclr/vm/siginfo.cpp` ~linha 5001) e quais
instruções IL passam a falhar quando `T` é ByRefLike (`box`, `newarr`, `stsfld` etc.).

### 5. Comparação com erasure (JVM) — conhecimento geral, não está no repositório

- Na JVM, `List<String>` e `List<Integer>` compartilham a mesma classe em runtime; os argumentos
  são apagados (type erasure), e o compilador insere casts. Não há genéricos sobre primitivos
  (usa-se `Integer` boxed).
- Há também o artigo citado como base do design do .NET: Kennedy & Syme, *Design and
  Implementation of Generics for the .NET Common Language Runtime* (referenciado em
  `type-loader.md`, seção de referências, link `generics-design`).

## Por quê

Razões que o repositório sustenta de forma direta:

1. **Desempenho com value types**: reificar permite código especializado por struct, sem boxing
   (a doc de sharing diz que o compartilhamento só é feito onde "tamanho/propriedades/layout"
   são idênticos — referências —, `shared-generics.md`, intro).
2. **Verificabilidade e segurança de tipo em runtime**: o carregador valida constraints e o
   casting pode considerar argumentos (variância).
3. **Reflexão e serialização fiéis**: o tipo fechado existe como objeto de primeira classe.
4. **Uso de `Span<T>` e afins em genéricos** (`byreflike-generics.md`): só faz sentido se o runtime
   sabe em tempo de carga que `T` é ByRefLike e pode validar cada IL afetado.

Motivações históricas (decisão de projeto de 2001-2004) não estão nos arquivos lidos; evite
atribuí-las.

## Trade-offs

- **Custo de memória/tempo de carga**: mais `MethodTable`s e `EEClass`es. A doc de sharing diz
  explicitamente que o compartilhamento existe "para reduzir a memória e o tempo de JIT"
  (`shared-generics.md`).
- **Complexidade de implementação**: dicionários genéricos, layouts de dicionário, expansão
  dinâmica de dicionários e helpers (`JIT_GenericHandleClass`/`Method`) só existem por causa da
  reificação + sharing (ver `03`).
- **Inchaço de código para value types**: cada `Foo<struct>` gera código novo. Em AOT isso
  pesa no tamanho do binário.
- **Interoperabilidade entre linguagens**: todas precisam entender genéricos reificados; adicionar
  recursos (ex.: ByRefLike como argumento) exige tocar em metadado, IL, JIT, trimmer e outras
  linguagens (`byreflike-generics.md`, lista de impactos: ILDasm, Reflection.Metadata, Cecil, ILLink,
  F#, C++/CLI).
- **Menos flexibilidade de variância "de graça"** que o erasure (arrays/covariância declaradas
  no site de uso) — o .NET exige variância declarada no site de declaração (ver `05`).
- **Compilação AOT**: precisa descobrir estaticamente todas as instanciações necessárias ou
  recorrer a dicionários/canônico universal; instanciações descobertas só em runtime (reflexão
  `MakeGenericType`) exigem JIT ou pré-geração (inferência a partir da existência de
  `__UniversalCanon` e dos dicionários; não aprofundado aqui).

## Como aplicar na sua linguagem

1. **Escolha o modelo cedo**: erasure é mais simples (um corpo de código, sem dicionários), mas
   proíbe especializar para tipos de valor e perde informação em runtime. Reificação com
   especialização total é rápida, mas explode código. O .NET fica no meio: reificação de tipos +
   compartilhamento de código para referências.
2. **Se reificar, modele identidade de instanciação**: uma tabela `(definição genérica, [args])
   → tipo` com garantia de unicidade (como o hash por módulo carregador).
3. **Separe "tipo reificado" de "corpo de código canônico"**. Você pode ter muitos tipos e poucos
   corpos de código.
4. **Defina a regra de sharing pelo critério de layout**: se todos os argumentos têm o mesmo
   tamanho/representação e as operações no corpo não dependem do argumento além de loads de
   metadados, compartilhe.
5. **Planeje o "contexto genérico" do código compartilhado**: ou um parâmetro oculto (dicionário)
   ou o `this`. O .NET usa MethodDesc/MethodTable como contexto (`shared-generics.md`).
6. **Se sua linguagem é compilada AOT**, planeje o que acontece com instanciações criadas
   dinamicamente.
7. **Considere um modo híbrido**: por exemplo, reificar apenas para tipos de valor e apagar
   para tipos de referência (o que na prática o .NET faz em nível de código, mantendo identidade
   de tipo).

### Tabela comparativa (síntese)

| Questão | Erasure (JVM, conhecimento geral) | Reificação (.NET, confirmado no repo) |
|---|---|---|
| Tipo em runtime de `List<int>` vs `List<string>` | Mesma classe | `MethodTable`s distintos, únicos por hash do módulo carregador |
| Genéricos sobre primitivos/structs | Não (boxing) | Sim, código especializado, sem sharing |
| Genéricos sobre referências | 1 corpo + casts | 1 corpo canônico (`__Canon`) + dicionário |
| `typeof(T)`, `new T()`, `T[]` em runtime | Impossível/limitado | Possível (via dicionário/handles) |
| Custo | Menor memória/startup | Mais tipos, dicionários, helpers |

### Exemplo de operações que dependem de reificação

```csharp
static T[] Make<T>(int n) => new T[n];       // precisa do TypeHandle de T[] em runtime
static bool Is<T>(object o) => o is T;       // checagem de tipo exata/variante
static string Name<T>() => typeof(T).Name;   // lookup no dicionário
```

Em código compartilhado (`T` = referência) cada uma dessas operações vira um *generic lookup*
(slot do dicionário) e não uma constante embutida (ver `03`).

### Checklist de decisões

- [ ] Quero código especializado para tipos de valor? Então preciso de reificação.
- [ ] Qual é a chave de identidade de uma instanciação (definição + argumentos)?
- [ ] Qual é a política de compartilhamento de código e qual a forma canônica?
- [ ] Como instanciações criadas por reflexão são suportadas em AOT?

## Referências

- `docs/design/coreclr/botr/type-loader.md` (seções 2.2, 2.2.2)
- `docs/design/coreclr/botr/shared-generics.md`
- `docs/design/features/byreflike-generics.md`
- `docs/design/coreclr/botr/managed-type-system.md` (type system gerenciado para AOT)
- `src/coreclr/vm/methodtable.cpp` (`CanCastByVarianceToInterfaceOrDelegate`)
- `src/coreclr/vm/typedesc.h` (`TypeVarTypeDesc`)
- `src/coreclr/vm/generics.cpp`, `src/coreclr/vm/genericdict.cpp`
- `src/coreclr/System.Private.CoreLib/src/System/__Canon.cs`
- `src/coreclr/tools/Common/TypeSystem/Canon/CanonTypes.cs`
