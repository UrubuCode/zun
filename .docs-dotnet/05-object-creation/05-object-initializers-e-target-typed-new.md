# Object initializers e target-typed new (açúcar do compilador)

## Mecanismo no .NET

### Importante: é açúcar do compilador, não feature do runtime

Nenhum dos recursos abaixo tem representação especial no IL nem no runtime. O compilador C#
(Roslyn, repositório `dotnet/roslyn`, **não presente em `dotnet/runtime`**) reescreve tudo
para `newobj` + chamadas de setter/`Add`. O que se segue sobre a tradução vem da
especificação C#; o que é verificável neste repo é que **o runtime só vê `newobj`, `call`,
`stfld`** (veja arquivo 01 e `src/coreclr/inc/corinfo.h`).

### Object initializer

```csharp
var p = new Person { Name = "Ana", Age = 30 };
```

é traduzido aproximadamente para:

```csharp
var tmp = new Person();     // newobj Person::.ctor()
tmp.Name = "Ana";           // callvirt set_Name
tmp.Age  = 30;
var p = tmp;                // só atribuído ao destino depois de tudo
```

Pontos:
- O objeto é criado e inicializado em um **temporário**; a variável/campo de destino só vê o
  objeto depois de todos os setters (evita observar objeto parcial por outra thread via
  destino, mas **não** protege contra os setters publicarem `this`).
- Se um setter lança, o destino **não** é atribuído (a variável anterior fica intacta).
- Funciona com campos e propriedades acessíveis; usa o ctor sem parâmetros ou
  `new T(args) { ... }`.
- Inicializadores aninhados: `new A { B = { X = 1 } }` não cria `B`: modifica o `B`
  existente (`a.B.X = 1`).

### Collection initializer

```csharp
var l = new List<int> { 1, 2, 3 };   // newobj + callvirt Add(1), Add(2), Add(3)
var d = new Dictionary<string,int> { ["a"] = 1 };   // indexer initializer: set_Item
```

Requer que o tipo implemente `IEnumerable` e tenha um método `Add` acessível (também via
extension method). Collection expressions (C# 12, `[1,2,3]`) vão além: o compilador escolhe
a estratégia (inline array, `CollectionBuilderAttribute`, `Span`) segundo o tipo-alvo.

### `init` accessors e `required`

- `init` (C# 9): setter marcado com o modreq `System.Runtime.CompilerServices.IsExternalInit`
  — a restrição “só durante construção” é imposta **pelo compilador**, não pelo runtime.
  A definição do tipo `IsExternalInit` está em `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IsExternalInit.cs`.
- `required` (C# 11): atributo `RequiredMemberAttribute` + `SetsRequiredMembersAttribute`;
  o compilador exige que o site de criação atribua o membro. Novamente checagem apenas em
  compile‑time.
- `record`: gera `Clone`/`with` expressions via `<Clone>$` e ctor de cópia.

### Target‑typed `new`

```csharp
List<int> l = new();           // C# 9: tipo inferido do destino
Person[] arr = { new(), new() };
void F(Point p); F(new(1, 2));
```

O compilador infere o tipo a partir do **tipo‑alvo** da expressão e emite exatamente o
mesmo `newobj` que `new List<int>()`. Não há mudança de runtime.

### `with` expression, deconstruction, tuples

- `p with { X = 1 }` → chama o método sintetizado de clone e depois setters `init`.
- Tuple literal `(1, "a")` → `ValueTuple<int,string>`; criação por `newobj` ou, em muitos
  casos, `initobj` + `stfld` no struct na pilha.

### Relação com o runtime

Para o runtime e o JIT, todas as formas são `newobj` + `callvirt`/`stfld`. O JIT pode
fazer *inlining* dos setters triviais e *escape analysis* (object stack allocation habilitado
em .NET 9/10 para casos simples) — ver `src/coreclr/jit/objectalloc.cpp`
Nesse caso um `new Person { ... }` que não escapa pode virar campos em
locais.

## Por quê

- **Ergonomia** sem custo de runtime: mutáveis fáceis de construir sem 10 overloads de
  ctor.
- **Atribuição atômica ao destino**: a semântica do temporário garante que quem lê a
  variável de destino nunca vê objeto meio configurado *via aquela variável*.
- **Mantém o runtime pequeno**: a linguagem pode evoluir (init, required, collection
  expressions) sem mudar o formato do IL/metadata — exceto por atributos/modreqs.
- **Target typing** reduz repetição sem perder tipagem estática.

## Trade-offs

- Como o objeto ainda é construído em duas fases (ctor + setters), **invariantes** da classe
  não podem ser garantidas dentro do ctor – daí `required`, que só ajuda no compile‑time e
  é ignorado por reflexão/serialização.
- Initializers só funcionam bem para tipos mutáveis ou com `init`; geram dependência de
  nomes públicos de propriedades (quebra de API ao renomear).
- Garantias (`init`, `required`) são só de compilador: outro compilador/linguagem pode
  ignorar (o modreq protege em parte).
- Debugging: stack traces e breakpoints em setters dentro de expressões grandes confundem.
- Target-typed `new` dificulta leitura quando o tipo-alvo é distante.

## Como aplicar na sua linguagem

1. Prefira **lowering no front-end** (desugaring) para initializers: transforme em `tmp =
   new T(); tmp.f = ...; dest = tmp`. Isso mantém o IR/VM mínimo.
2. Defina que o destino só é atribuído ao final (semântica do temporário) e documente o que
   acontece se um setter lança.
3. Se quiser imutabilidade com sintaxe de objeto, adicione `init`/`required` **com
   representação em metadados** que outros front-ends reconheçam, ou imponha em runtime para
   garantias reais.
4. Implemente inferência bidirecional para `new()` — o tipo vem do contexto (retorno,
   parâmetro, atribuição). Cuidado com ambiguidade com overloads.
5. Considere named arguments no ctor como alternativa aos initializers (Kotlin, Swift,
   Python): evita construção em duas fases e permite campos imutáveis de verdade.
6. Deixe o otimizador tratar o padrão `new + setters` como uma unidade para escape analysis.

## Detalhes adicionais

### Tabela de açúcares e sua tradução

| Sintaxe | Tradução conceitual | O runtime vê |
|---|---|---|
| `new T { A = 1 }` | `tmp = new T(); tmp.A = 1;` | `newobj` + `callvirt set_A` |
| `new List<int> { 1, 2 }` | `new List<int>(); Add(1); Add(2);` | `newobj` + `callvirt Add` |
| `new()` (target typed) | tipo do destino | `newobj` normal |
| `with { }` | clone + setters init | chamada ao método de clone + setters |
| `[1, 2, 3]` | depende do tipo-alvo | `newarr` / `InlineArray` / Span |
| `required` | checagem no ponto de criação | atributo em metadados |

### Cuidados de semântica

- Avaliação em ordem textual: expressões nos initializers são avaliadas na ordem em que
  aparecem, e efeitos colaterais acontecem antes da atribuição ao destino.
- Se o initializer usa `await`, o compilador transforma em máquina de estados e o temporário
  vira campo da máquina.
- Em structs, o initializer opera sobre o local in-place (`ldloca` + `stfld`), sem cópias.
- Setters `init` só são permitidos em initializer, ctor, outro `init` ou `with`.

### Lição para o design

O contrato persistente entre compilador e biblioteca está nos atributos (`IsExternalInit`,
`RequiredMemberAttribute`, `CollectionBuilderAttribute`, em
`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/`); a tradução do
código é detalhe de implementação. Defina cedo quais atributos/anotações de metadados seu
compilador publicará.

## Referências

- Roslyn (`dotnet/roslyn`) — lowering de object/collection initializers (fora deste repo).
- Especificação C# 9/10/11/12 – “Object creation expressions”, “target-typed new”,
  “init accessors”, “required members”.
- `src/coreclr/inc/corinfo.h` — o runtime só conhece helpers de `NEW*`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/` —
  `IsExternalInit`, `RequiredMemberAttribute`, `CollectionBuilderAttribute` (nomes
  confirmados no repo).
- `src/coreclr/jit/` — escape analysis / object stack allocation (`objectalloc.cpp`).
