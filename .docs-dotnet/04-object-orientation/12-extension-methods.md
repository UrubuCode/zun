# Extension methods (despacho estático)

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### Um método estático com marca de metadado

Um extension method é, no IL, um método `static` comum numa classe estática. A "extensão" é:

1. um atributo `System.Runtime.CompilerServices.ExtensionAttribute` no método (e, por exigência do compilador, na classe e no assembly que o contém), declarado em `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ExtensionAttribute.cs`:

```csharp
/// Indicates that a method is an extension method, or that a class or assembly contains extension methods.
[AttributeUsage(AttributeTargets.Method | AttributeTargets.Class | AttributeTargets.Assembly)]
public sealed class ExtensionAttribute : Attribute { }
```
(repo)

2. a sintaxe `this` no primeiro parâmetro (C#), que o compilador traduz em ExtensionAttribute.

O *runtime* não sabe de nada disso. Não há categoria especial de `MethodDesc` (`docs/design/coreclr/botr/method-descriptor.md` lista IL, Instantiated, FCall, PInvoke, EEImpl, Array, ComInterop, Dynamic, repo), nem slot de vtable, nem consulta no `MethodTableBuilder`. A chamada `x.Foo(y)` é resolvida pelo *compilador* e emitida como `call Classe.Foo(x, y)` (C#/ECMA).

### Exemplos no repo

- LINQ: `public static partial class Enumerable` (`src/libraries/System.Linq/src/System/Linq/`), p.ex. em `Where.cs`: `public static IEnumerable<TSource> Where<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)`. O corpo faz a checagem de nulo *explicitamente* (`if (source is null) ThrowHelper.ThrowArgumentNullException(ExceptionArgument.source);`, ~14-16): como a chamada é estática, `null.Where(...)` não dá NullReferenceException do despacho; a validação é responsabilidade do método (repo).
- Spans: `src/libraries/System.Private.CoreLib/src/System/MemoryExtensions.cs` define `Contains<T>(this Span<T> span, T value) where T : IEquatable<T>?` (~318) e muitos outros: adiciona operações a tipos `ref struct` que não podem implementar interfaces genéricas facilmente, e mantém `Span<T>` pequeno (repo).
- "Extension members" de C# 14 (blocos `extension(...)`): o repo já usa a sintaxe em polyfills internos, ex. `src/libraries/Common/src/Polyfills/ArrayPolyfills.cs`: `internal static class ArrayPolyfills { extension(Array) { public static int MaxLength => ...; public static void Clear(Array array) {...} } }`, que adiciona membros *estáticos* e propriedades a `Array` (repo). Isso estende o conceito para propriedades e estáticos (a forma de lowering não foi verificada por mim; trato-a como C#/ECMA).

### Despacho: sempre estático, nunca virtual

Resolução (C#/ECMA, descrição das regras de linguagem):

1. Se o tipo (ou seus pais) tem um método de instância aplicável, ele *vence* sempre; extensão só entra se não houver membro de instância.
2. Depois, procuram-se classes estáticas com extensões nos namespaces importados (`using`) a partir do escopo mais interno.
3. O primeiro `receiver` é convertido por conversão de identidade, referência ou boxing (não por conversões definidas pelo usuário).
4. Ligação em tempo de compilação com o tipo *estático* da expressão: o método chamado não muda se o objeto real for de um subtipo. Não participa de vtable, interface map ou VSD (arquivos 02 e 03).

Consequências: pode ser aplicado a `null`; pode ser aplicado a interfaces (`IEnumerable<T>`), struct, enum, delegate; e não pode acessar membros privados do receptor.

### Extensões sobre interfaces como "default methods" precoce

LINQ prova o padrão: `IEnumerable<T>` tem uma interface minúscula (`GetEnumerator`), e ~dezenas de operações moram como extensões em `Enumerable`. Antes de DIM (arquivo 01), era a única forma de adicionar comportamento "a todas as implementações de uma interface". Diferença-chave com DIM: extensão não é virtual, então o tipo concreto não pode sobrescrever (mas as implementações de LINQ retornam classes iteradoras especializadas, ex. `IEnumerableWhereIterator<TSource>` em `Where.cs`, e podem otimizar por tipo do argumento).

### Versionamento e conflitos

- Adicionar um membro de instância a um tipo com o mesmo nome/assinatura de uma extensão existente *muda silenciosamente* qual método é chamado em recompilações (membro de instância ganha). Sem recompilar, binários antigos continuam chamando a estática (já resolvida no IL) (C#/ECMA, consequência direta do despacho estático).
- Duas extensões equivalentes em namespaces importados causam erro de ambiguidade em tempo de compilação.

## Por quê

1. **Estender tipos que você não possui** (sealed, de outra biblioteca, interfaces) sem herança nem wrapper.
2. **Sintaxe fluente / encadeamento** (`xs.Where(...).Select(...)`) sem inchar as interfaces base.
3. **Sem custo de runtime nem mudança no CLR**: é uma chamada estática. O CLR e o formato de metadado não precisaram evoluir (consistente com a ausência de qualquer tratamento no VM).
4. **Manter interfaces pequenas**: `IEnumerable<T>` tem 1 método útil; LINQ não obriga implementadores a fazer nada.
5. **Descoberta por IDE** via `using`.
6. **Evitar quebrar contratos**: adicionar uma extensão não muda a ABI do tipo.

## Trade-offs

- **Não polimórfico**: um tipo não pode especializar uma extensão (em contraste com DIM ou virtual). O LINQ recorre a otimizações internas por tipo do argumento (inferência).
- **Resolução por escopo** (namespaces importados) torna o significado de `x.F()` dependente de `using`; refatorações e conflitos surgem.
- **Menor prioridade que membros**: adicionar um método ao tipo muda a semântica de chamadas existentes (hijacking silencioso) no recompile.
- **Sem acesso a membros privados**; limitado à API pública do receptor.
- **Descobribilidade ambígua**: a origem do método não é óbvia sem ferramenta.
- **Genéricos e constraints**: assinaturas como `where T : IEquatable<T>?` e sobrecargas por `Span<T>` / `ReadOnlySpan<T>` multiplicam variantes (ver `MemoryExtensions.cs`, repo).
- **`null` receiver**: o método precisa validar; é fácil esquecer.
- **Não resolve propriedades/estáticos** até o novo recurso de blocos `extension` (C# 14).
- **Reflexão/dinâmico**: `dynamic` não vê extensões; reflexão só vê o método estático original (C#/ECMA).

## Como aplicar na sua linguagem

1. **Implemente como açúcar de front-end**: `x.f(a)` => `f(x, a)` (UFCS, Uniform Function Call Syntax). Nenhuma mudança no formato de binário nem no despacho. Marque o método (atributo ou flag) apenas para o compilador/ferramentas saberem que podem ser chamados com sintaxe de ponto.
2. **Decida a regra de precedência**: membros de instância ganham das extensões (como C#) é a escolha mais previsível. Documente o efeito de adicionar membros depois.
3. **Escopo de visibilidade**: importar o módulo = habilitar extensões. Defina a política de ambiguidade (erro) e se há ordenação por proximidade.
4. **Receptor genérico/de interface**: permita extensões sobre interface/trait e sobre tipos de valor (essencial para o padrão LINQ). Defina conversões permitidas no receptor (identidade, subtipo, boxing).
5. **Não finja polimorfismo**: se quer extensões *virtuais*, ofereça DIM/métodos padrão em traits (arquivo 01). Mantenha a distinção clara: estático = resolvido no compilador.
6. **Null/nil receiver**: decida se o receptor pode ser nulo e o que isso significa. No .NET pode; o método deve checar (veja `Where.cs`).
7. **Extensão de estáticos e propriedades**: se quiser, planeje desde o início uma sintaxe de bloco (`extension(T) { ... }`) como o repo já faz em polyfills C# 14, para agrupar membros e evitar repetir `this`.
8. **Versionamento**: resolva em tempo de compilação e grave a ligação (qual função) no binário, como o IL faz. Assim binários antigos não mudam de comportamento sem recompilar.
9. **Otimizações específicas por tipo** (como LINQ faz com `is IList<T>`) são aceitáveis dentro da implementação, mas mantenha uma API estável.
10. **Ferramentas**: forneça "go to definition" e indicação visual de que o método é extensão.

Lowering de referência:

```
// fonte
fn ext(self: Lista<T>) -> int { ... }     // marcada como extensão
xs.ext()
// IR
call ext(xs)                              // estática, ligada em compilação
```

## Resumo dos fatos confirmados

- `ExtensionAttribute` é um atributo selado sem membros, aplicável a método, classe e assembly.
- O CLR não tem tratamento especial; a chamada é estática.
- LINQ (`Enumerable`) e `MemoryExtensions` são os usos de referência no repo.
- `Where<TSource>` verifica `source` e `predicate` contra `null` manualmente.
- O repo já usa blocos `extension(...)` (C# 14) em polyfills internos.

## Ordem de resolução (C#/ECMA)

1. Membros de instância do tipo e de seus pais.
2. Extensões em namespaces mais internos.
3. Extensões em namespaces importados por `using`.
4. Ambiguidade entre candidatas equivalentes: erro de compilação.

## Comparação com alternativas

| Mecanismo | Despacho | Pode especializar por tipo? | Atravessa assemblies? |
|---|---|---|---|
| Extension method | estático | não | sim |
| DIM (default interface method) | virtual | sim (implementador pode sobrescrever) | sim (dentro do contrato) |
| Método virtual na classe | virtual | sim | exige herança |
| Partial method | estático (mesma compilação) | não | não |
| Wrapper/decorator | depende | sim | sim |

## Perguntas de design

- A sintaxe `x.f()` pode chamar uma função livre (UFCS)?
- A extensão pode ser aplicada a `null`/`nil`?
- Há prioridade de membros de instância sobre extensões?
- Como se importa e como se evita ambiguidade?
- Extensões podem ser genéricas sobre traits?
- Extensões podem adicionar propriedades, operadores e estáticos?
- Extensões de interface criam a ilusão de herança de implementação: está claro que não são virtuais?
- A documentação distingue extensão de membro nativo?

## Glossário

- Extension method: método estático chamável com sintaxe de instância.
- Receptor: o primeiro argumento (`this`) do extension method.
- Classe de extensão: classe estática que contém extension methods.
- UFCS: Uniform Function Call Syntax, a ideia geral por trás da sintaxe.
- Hijacking: um membro de instância novo passa a ganhar da extensão antiga após recompilação.
- Extension block: sintaxe nova do C# 14 que agrupa membros de extensão por tipo.
- Polyfill: implementação de uma API ausente em versões antigas, aqui feita com extensões.
- Fluent API: encadeamento de chamadas que cada extensão devolve.
- ThrowHelper: utilitário do repo que centraliza o lançamento de exceções para manter métodos pequenos.

## Nota final

- O ponto central: extensão é uma conveniência de sintaxe, não um mecanismo de polimorfismo.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ExtensionAttribute.cs`.
- `src/libraries/System.Linq/src/System/Linq/Where.cs` (e demais arquivos de `Enumerable`).
- `src/libraries/System.Private.CoreLib/src/System/MemoryExtensions.cs`.
- `src/libraries/Common/src/Polyfills/ArrayPolyfills.cs` (sintaxe `extension(...)`).
- `docs/design/coreclr/botr/method-descriptor.md` (sem categoria especial para extensão).
- `docs/design/specs/Ecma-335-Augments.md` (default interface methods, comparação com DIM).
