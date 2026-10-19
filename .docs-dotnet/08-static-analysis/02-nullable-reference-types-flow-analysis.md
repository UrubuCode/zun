# Nullable reference types — análise de fluxo de nulidade

> Importante: o **compilador** C# (Roslyn) que implementa a análise de fluxo de nulidade **não está** neste repositório. O que o repositório `dotnet/runtime` contém e é confirmável é (a) a *adoção* da feature em todo o código de bibliotecas, (b) o conjunto de atributos de anotação que alimentam a análise, (c) a representação em metadados (arquivo 07) e (d) como as ferramentas do repo (ILLink analyzer) lidam com nulidade em suas próprias análises. Onde descrevo o algoritmo interno de Roslyn, marco como conhecimento geral, não verificado neste repo.

## Mecanismo no .NET

### 1. Habilitação por projeto (confirmado)

A feature é controlada pela propriedade MSBuild `Nullable`. No repositório:

- `Directory.Build.props` (raiz, ~linha 404): `<Nullable Condition="'$(Nullable)' == '' and '$(Language)' == 'C#'">annotations</Nullable>` — o padrão global é `annotations` (apenas lê anotações, não emite avisos de fluxo).
- `src/libraries/Directory.Build.props` (linhas 62-63): `enable` para projetos de produção e `annotations` para projetos de teste (`IsTestProject`).
- `src/libraries/System.Private.CoreLib/src/System.Private.CoreLib.Shared.projitems` (linha 13) e `src/coreclr/System.Private.CoreLib/System.Private.CoreLib.csproj` (linha 44): `<Nullable>enable</Nullable>`.
- Comentário em `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/Dataflow/README.md`: arquivos portados do ILLink recebem `#nullable enable` "porque nullable ainda não está globalmente habilitado no ILC" — ou seja, a adoção é **incremental, arquivo a arquivo**.

Os modos (`disable`, `annotations`, `warnings`, `enable`) são do compilador; o repo só mostra o uso de `annotations` e `enable`.

### 2. Anotações de contrato (confirmado)

`src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/NullableAttributes.cs` define os atributos que permitem expressar contratos que o sistema de tipos `T` / `T?` sozinho não captura:

| Atributo | Significado (conforme os doc-comments/uso) |
|---|---|
| `AllowNullAttribute` | Entrada aceita `null` mesmo que o tipo seja não-nulo |
| `DisallowNullAttribute` | Entrada não aceita `null` mesmo que o tipo seja nulo |
| `MaybeNullAttribute` | Saída pode ser `null` mesmo que o tipo seja não-nulo (ex.: `T` genérico) |
| `NotNullAttribute` | Saída nunca é `null` mesmo que o tipo seja nulo |
| `MaybeNullWhenAttribute(bool)` | Saída pode ser nula quando o método retorna o `bool` dado (padrão `TryGet`) |
| `NotNullWhenAttribute(bool)` | Saída não é nula quando o método retorna o `bool` dado |
| `NotNullIfNotNullAttribute(string)` | Retorno não nulo se o parâmetro nomeado for não nulo |
| `DoesNotReturnAttribute` | Método nunca retorna (lança/termina) |
| `DoesNotReturnIfAttribute(bool)` | Não retorna se o parâmetro `bool` tiver o valor dado (ex.: `Debug.Assert`) |
| `MemberNotNullAttribute` | Após a chamada, os membros listados são não-nulos |
| `MemberNotNullWhenAttribute` | Idem, condicionado ao retorno `bool` |

(Todos listados com `sealed class ...Attribute` no arquivo, linhas 14-162.)

O ponto de design: **o estado de nulidade é um fato de fluxo**, e estes atributos são "pós-condições" e "pré-condições" que o chamador pode usar sem ver o corpo do método. É análise **intraprocedural** com contratos para fronteiras entre métodos.

### 3. Modelo de análise (conhecimento geral, não verificado no repo)

Em Roslyn, cada expressão tem um *estado de fluxo* — "não nulo" ou "talvez nulo" — separado do *tipo declarado* (anotado `?` ou não). A análise percorre o grafo de fluxo de controle, atualiza o estado em atribuições, comparações (`x != null`, `is null`, `is not null`), `??`, chamadas com atributos acima, e junta estados nas junções (um `maybe-null` num dos ramos torna a junção `maybe-null`). Os avisos (CS86xx/CS8600 etc.) são emitidos quando um valor `maybe-null` é desreferenciado ou atribuído a local não-nulo. Isso não altera o IL gerado: nulidade em tempo de execução continua a mesma.

### 4. Nulidade dentro de uma análise do repo: ILLink

O trimmer também precisa raciocinar sobre nulos, no seu próprio domínio de valores. Em `src/tools/illink/src/ILLink.Shared/TrimAnalysis/` existem:

- `NullValue.cs` — o valor abstrato `null`.
- `NullableSystemTypeValue.cs`, `NullableValueWithDynamicallyAccessedMembers.cs`, `NullableUnwrappedGenericParameterValue.cs` — representam `Nullable<T>` e o desembrulhar de `typeof(Nullable<T>)`.
- `RuntimeTypeHandleForNullableSystemTypeValue.cs` e `RuntimeTypeHandleForNullableValueWithDynamicallyAccessedMembers.cs`.

Aqui "nullable" se refere sobretudo a `Nullable<T>` (tipo valor) em padrões de reflexão, mas `NullValue` mostra que o lattice de valores do analisador inclui `null` como elemento próprio — o mesmo princípio de "lattice de nulidade" usado em Roslyn.

Também em `src/tools/illink/src/ILLink.RoslynAnalyzer/NullableAttributes.cs` o próprio analisador define cópias internas dos atributos de nulidade, porque ele é compilado para `netstandard2.0` (onde `System.Diagnostics.CodeAnalysis.*` não existe).

### 5. Como o runtime *se defende* em runtime

A nulidade anotada **não é imposta em tempo de execução**: um `string` não-anulável pode receber `null` via reflexão ou código sem anotações. Por isso as bibliotecas ainda usam guardas explícitas como `ArgumentNullException.ThrowIfNull` onde a API é pública. (Padrão geral do .NET; o ponto aqui é que anotação ≠ garantia.) A coexistência de código "oblivious" (sem anotações, `Nullable=disable`) e anotado é parte do design — daí o modo `annotations` como padrão do repo.

## Por quê

- **Eliminar uma classe de bugs (NullReferenceException) em tempo de compilação** sem mudar o runtime nem o formato dos tipos: `string` e `string?` são o mesmo tipo em IL.
- **Compatibilidade retroativa**: bilhões de linhas existentes tinham `string` significando "talvez nulo". Introduzir nulidade como *anotação + análise* e não como sistema de tipos novo evita quebrar binários.
- **Adoção gradual**: por projeto (`Nullable` property), por arquivo (`#nullable`), por trecho. O repo mostra todas as camadas.
- **Contratos expressivos**: padrões como `TryGetValue(key, out value)` não cabem em `T`/`T?`; daí `NotNullWhen`, `MaybeNullWhen`, `MemberNotNull`.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Nulidade como anotação sem representação em tipo de runtime | Zero custo em runtime; binários compatíveis | Garantia é só de "melhor esforço": reflexão, código antigo e `!` contornam |
| Avisos (não erros) por padrão | Migração viável | Equipes ignoram avisos; precisa de `TreatWarningsAsErrors` |
| Análise de fluxo intraprocedural + atributos | Rápida, local, incremental | Falsos positivos em invariantes complexos (ex.: campos inicializados fora do ctor) → operador `!` e `MemberNotNull` |
| Genéricos: `T` sem restrição pode ser nulo ou não | Flexível | Precisa de `[MaybeNull]`/`[AllowNull]`/`T?` e regras sutis (`notnull` constraint) |
| Atributos como biblioteca (em CoreLib) | Linguagem não precisa de sintaxe nova para cada contrato | Cada projeto `netstandard2.0` precisa de cópias (como `ILLink.RoslynAnalyzer/NullableAttributes.cs`) |

## Como aplicar na sua linguagem

1. **Decida cedo**: nulidade *no sistema de tipos* (como Kotlin/Swift: `T` vs `T?` com representação distinta ou checada) ou *anotação sobreposta* (modelo .NET). Para uma linguagem nova sem legado, a primeira opção dá garantias reais; o modelo .NET só se justifica por compatibilidade.
2. **Separe "tipo declarado" de "estado de fluxo"**: mesmo que `x: T?`, depois de `if (x != null)` o estado é "não nulo". Implemente a análise como interpretação abstrata sobre o CFG com lattice `{NotNull, MaybeNull}` (e eventualmente `Unknown/Oblivious`).
3. **Forneça atributos/anotações de contrato** equivalentes a `NotNullWhen`, `MemberNotNull`, `DoesNotReturn`: sem eles, todo `TryParse`-like gera falso positivo. Faça-os parte da biblioteca padrão, não da gramática.
4. **Modele `DoesNotReturn`**: é o que torna `throw helper` e `assert` úteis ao fluxo (confirmado no conjunto de atributos: `DoesNotReturn`, `DoesNotReturnIf`).
5. **Inclua um estado "oblivious"** para código sem anotações, para permitir adoção incremental; ofereça modos como `annotations`/`enable` por projeto.
6. **Registre a anotação em metadados** (arquivo 07) para que bibliotecas compiladas separadamente transmitam contratos de nulidade.
7. **Não confie nas anotações para segurança de memória**; trate-as como ferramenta de qualidade. Se quiser garantia, imponha em runtime nas fronteiras (reflexão, FFI).
8. **Teste o analisador com casos pequenos de fluxo** (padrão do repo para analisadores: `src/tools/illink/test/ILLink.RoslynAnalyzer.Tests/`).

## Apêndice: exemplo de fluxo e modelo de implementação (conhecimento geral + atributos confirmados)

### Exemplo de estados ao longo do fluxo

```csharp
string? s = Obter();            // estado: MaybeNull (tipo declarado: anulável)
if (s != null)
{
    Usar(s);                    // estado dentro do if: NotNull
}
else
{
    s = "x";                    // atribuição de literal -> NotNull
}
Usar(s);                        // junção: NotNull(then) ∧ NotNull(else) = NotNull

if (!string.IsNullOrEmpty(s2))  // IsNullOrEmpty tem [NotNullWhen(false)] no parâmetro
    Usar(s2);                   // NotNull graças ao atributo de contrato
```

A última linha só funciona porque o contrato é declarado com `NotNullWhenAttribute` (definido em `NullableAttributes.cs`). Sem atributos, a análise não consegue "ver" dentro de `IsNullOrEmpty`.

### Pseudocódigo do lattice

```text
Estado = { NotNull, MaybeNull, Oblivious }
junção(a, b):  se a == b: a
               senão se Oblivious em {a,b}: Oblivious        # decisão de projeto
               senão: MaybeNull
transferência:
    x = null                -> x := MaybeNull
    x = <literal/new>       -> x := NotNull
    x = y                   -> x := estado(y)
    if (x != null) {...}    -> então: x := NotNull; senão: x := MaybeNull
    chamada com [NotNullWhen(b)] em parâmetro p, se resultado == b: p := NotNull
    [DoesNotReturn]         -> fim de caminho (ignorado na junção)
    [MemberNotNull(f)]      -> após a chamada: this.f := NotNull
```

Os nomes `NotNull`/`MaybeNull`/`Oblivious` correspondem aos três valores de `NullabilityState` no runtime (`Unknown`, `NotNull`, `Nullable` em `System/Reflection/NullabilityInfo.cs`): bom indício de que o modelo mental de três estados é o central.

### Perguntas de projeto que o .NET respondeu (e você precisa responder)

| Pergunta | Resposta .NET | Evidência no repo |
|---|---|---|
| Nulidade é parte do tipo em runtime? | Não | `string?` e `string` idênticos em IL; atributos só em metadados (arquivo 07) |
| Como adotar gradualmente? | Modos `annotations`/`enable`, `#nullable` | `Directory.Build.props` |
| Como expressar `TryGet`? | `[MaybeNullWhen(false)] out T value` / `[NotNullWhen(true)]` | `NullableAttributes.cs` |
| E campos inicializados no ctor? | `[MemberNotNull]` | idem |
| E `Debug.Assert(x != null)`? | `[DoesNotReturnIf(false)]` no parâmetro condição | idem |
| Genéricos? | `[MaybeNull]`, `[AllowNull]`, `T?`, restrição `notnull` | idem + conhecimento geral |
| Quem fiscaliza em runtime? | Ninguém automaticamente | guardas manuais nas APIs públicas |

### Anti-padrões a evitar no design

1. Tornar o aviso um erro hard desde o dia 1 em linguagem com código legado: bloqueia adoção.
2. Não prover `DoesNotReturn`: obriga `!` espalhado após `throw` helpers.
3. Não separar "estado de fluxo" do "tipo declarado": produz falsos positivos em `if (x != null)`.

## Referências

- `Directory.Build.props` (propriedade `Nullable`, ~linha 404)
- `src/libraries/Directory.Build.props` (linhas 62-63)
- `src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/NullableAttributes.cs`
- `src/libraries/System.Private.CoreLib/src/System.Private.CoreLib.Shared.projitems`; `src/coreclr/System.Private.CoreLib/System.Private.CoreLib.csproj`
- `src/tools/illink/src/ILLink.Shared/TrimAnalysis/NullValue.cs`, `NullableSystemTypeValue.cs`, `NullableValueWithDynamicallyAccessedMembers.cs`
- `src/tools/illink/src/ILLink.RoslynAnalyzer/NullableAttributes.cs`
- `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/Dataflow/README.md`
- Não presente no repo: implementação de Roslyn (`dotnet/roslyn`), especificação C# de nullable reference types.
