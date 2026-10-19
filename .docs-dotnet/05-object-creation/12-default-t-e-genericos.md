# default(T) e inicialização de genéricos

## Mecanismo no .NET

### `default(T)` em código genérico

Em `class Box<T> { T value = default; }` o compilador C# emite, para `default(T)` quando `T`
é parâmetro de tipo, a sequência IL: declarar um local temporário do tipo `T`, `ldloca`,
`initobj !T`, `ldloc`. O significado é **"zeros do tamanho de T"**: `null` se `T` for tipo de
referência, `0`/`false`/struct zerado se `T` for value type. A resolução ocorre quando o
tipo é **instanciado**: o .NET tem generics **reificados** (o runtime conhece `T` em
execução; `typeof(T)` funciona), diferente do apagamento de tipos do Java.

### Duas estratégias de código: especializado vs. compartilhado

Documento de design: `docs/design/coreclr/botr/shared-generics.md`.

- **Tipos valor** (`List<int>`, `List<MyStruct>`): o JIT gera **código especializado** por
  instanciação — o tamanho, layout e instruções de `T` são conhecidos, então `default(T)`
  vira um zero de N bytes embutido (e muitos testes tipo `typeof(T) == typeof(int)` são
  dobrados em tempo de JIT).
- **Tipos referência** (`List<string>`, `List<object>`): o código é **compartilhado**, uma
  única versão "canônica" (`__Canon`) para todas as instanciações com referências. Como
  todas as referências têm o mesmo tamanho e `default` é `null`, `default(T)` não precisa
  saber o `T` exato. Quando o código *precisa* do tipo exato (ex.: `new T[]`, `typeof(T)`,
  `new List<T>`), ele o carrega do **generic dictionary** (via contexto genérico: `MethodDesc`
  ou `MethodTable` + `m_pPerInstInfo`), ou por helper do runtime. O doc mostra o exemplo:
  `mov rcx, generic context` → `mov rcx, [rcx + offset ...m_pPerInstInfo]` → slot do
  dicionário com o type handle de `List<T>`.
- Em NativeAOT o mesmo conceito existe (código canônico + dicionários), implementado pelo
  compilador ILC (`docs/design/coreclr/botr/ilc-architecture.md`).

### Consequências para criação de objetos

- `new T()` (restrição `where T : new()`): o compilador emite
  `Activator.CreateInstance<T>()`. O JIT reconhece esse método como intrínseco
  (`NI_System_Activator_CreateInstance_T`, `src/coreclr/jit/importercalls.cpp`, linhas
  ~4052 e ~10923) e, quando `T` é conhecido (instanciação especializada), reduz a um
  `newobj`/`initobj` direto; em código compartilhado usa um caminho mais lento que
  descobre o construtor via o dicionário/runtime.
- `new T[n]` em genérico: aloca o array com o tipo de elemento resolvido pelo dicionário
  em código compartilhado.
- Campos estáticos de tipos genéricos são **por instanciação fechada** (`Foo<int>.X` ≠
  `Foo<string>.X`); `.cctor` roda por instanciação (arquivo 04).
- `Unsafe.SkipInit<T>(out T)`/`RuntimeHelpers.IsReferenceOrContainsReferences<T>()`:
  o JIT trata `IsReferenceOrContainsReferences` como intrínseco constante em instanciações
  especializadas (`importercalls.cpp` ~3692/3776), permitindo `AllocateUninitializedArray`
  só para tipos sem refs (arquivos 06/11).
- `EqualityComparer<T>.Default` (`src/libraries/System.Private.CoreLib/src/System/Collections/Generic/EqualityComparer.cs`)
  mostra o padrão: escolhe a implementação com base em `T` (struct implementando
  `IEquatable<T>`, enum, `Nullable`, ...) e o JIT dobra a escolha em tempo de compilação.

### Nullable e default

`Nullable<T>` (`default` = `HasValue == false`), `default` literal (C# 7.1) com tipo
inferido, e anotações `T?` / `[MaybeNull]` / `notnull` constraint (C# 8) para
"default pode ser null" em genéricos: `T?` em tipos não restritos significa "default(T)"
pois não distingue `class`/`struct`. Isso é só análise do compilador.

### Variância e restrições relevantes à criação

Restrições: `where T : new()`, `class`, `struct`, `unmanaged`, `notnull`, `allows ref
struct` (C# 13), interfaces estáticas abstratas (`T.Create()` — "static abstract members",
.NET 7) que permitem **fábricas genéricas sem reflexão**: `T.Zero`, `T.Parse(...)`.

## Por quê

- **Reificação**: preserva informação de tipo em runtime (reflexão, `typeof(T)`, arrays
  corretos) e permite otimizações por tipo valor sem boxing.
- **Compartilhamento para referências**: limita a explosão de código e de tempo de JIT;
  funciona porque todas as referências têm a mesma representação (ponteiro) e `default` = 0.
- **Especialização para valores**: desempenho de coleções de `int` sem boxing e sem
  indireção.
- **`default` = zeros** é consistente com o invariante geral da CLR (arquivo 09) e dispensa
  chamar construtores para `T` desconhecido.

## Trade-offs

- `where T : new()` em código compartilhado é lento (reflexão/caminho de runtime) e só
  permite ctor sem parâmetros.
- Explosão de código para muitas instanciações de value types (cada uma é JIT‑ada).
- Dicionários genéricos exigem lookups em tempo de execução e infraestrutura complexa
  (expansão dinâmica de dicionário, ver seção "Dynamic Dictionary Expansion" do doc).
- Genéricos sobre tipos valor + virtuais/interfaces geram *constrained calls* e *shared
  generics over interfaces* complicados.
- `default(T)` ambíguo para o programador (`null` ou 0?) e incompatível com invariantes
  de "não‑nulo" – daí o uso de `!`/`default!`.
- Em AOT, instanciações abertas via reflexão não são previsíveis (dificulta trimming).

## Como aplicar na sua linguagem

1. Decida a estratégia: **apagamento** (simples, sem custo de código; perde `T` em
   runtime), **monomorfização** (Rust/C++: rápido, mas explosão de código) ou **híbrido**
   (CLR: especializa valores, compartilha referências com dicionários).
2. Se híbrido, defina que `default(T)` = bits zero e que todas as referências têm o mesmo
   tamanho; assim o código canônico não precisa do `T` exato para zerar.
3. Forneça um mecanismo de **dicionário/contexto genérico** passado ao código canônico
   (argumento oculto ou carregado do `this`) para operações que precisam do tipo exato.
4. Em vez de `new T()` por reflexão, ofereça **membros estáticos abstratos em
   interfaces/traits** (`T.Create()`), que o compilador resolve estaticamente — mais rápido
   e mais expressivo.
5. Exponha ao otimizador predicados **constantes por instanciação** (`typeof(T)==typeof(X)`,
   `IsReferenceOrContainsReferences<T>`) para permitir ramos dobrados em tempo de JIT/AOT.
6. Defina como a nulabilidade se combina com `default(T)` (ex.: `T?` vs. `T` com `default`).
7. Isole estáticos por instanciação fechada (com `.cctor` próprio) e documente.

## Detalhes adicionais

### Exemplo: especialização vs. compartilhamento

```csharp
T Zero<T>() => default;
Zero<int>();      // código especializado
Zero<string>();   // código canônico (__Canon): retorna null
Zero<MyStruct>(); // especializado: zera N bytes
```

### Outras linguagens, para comparação (conhecimento geral)

| Linguagem | Estratégia | `default(T)` |
|---|---|---|
| Java | apagamento | sem `T` em runtime |
| C++/Rust | monomorfização | `T()` / `Default` explícito |
| C# | reificado, híbrido | `default` = bits zero |
| Go (1.18+) | GC shape stenciling + dicionários | `var zero T` |

### Detalhe de dicionários

O doc `shared-generics.md` descreve ponteiros de dicionário em tipos e métodos
(`m_pPerInstInfo`), slots de dicionário, layouts de dicionário e a **expansão dinâmica**
(seção "Dynamic Dictionary Expansion"). Esse é o custo oculto de compartilhar código.

### Perguntas de projeto

- `T` existe em runtime (reificado) ou é apagado?
- Instanciações de tipos valor são especializadas? Instanciações de referência compartilham código?
- Como o código compartilhado obtém o tipo exato (dicionário, argumento oculto)?
- `default(T)` = zero de bits? Como interage com nulabilidade?
- Como construir `T` genericamente: `new()`, membros estáticos abstratos, fábrica?
- Estáticos por instanciação ou compartilhados?

## Referências

- `docs/design/coreclr/botr/shared-generics.md` — canonical codegen, dicionários.
- `docs/design/coreclr/botr/type-system.md`, `ilc-architecture.md`, `type-loader.md`.
- `src/coreclr/jit/importercalls.cpp` — `NI_System_Activator_CreateInstance_T`,
  `NI_System_Runtime_CompilerServices_RuntimeHelpers_IsReferenceOrContainsReferences`.
- `src/libraries/System.Private.CoreLib/src/System/Collections/Generic/EqualityComparer.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Activator.cs` (`CreateInstance<T>`).
- ECMA‑335 §III.4.? `initobj`, §II.9 (generics) — externo ao repo.
