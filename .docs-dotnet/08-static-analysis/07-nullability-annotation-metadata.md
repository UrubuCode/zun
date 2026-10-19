# Metadados de anotação de nulidade (NullableAttribute)

## Mecanismo no .NET

### O problema de representação

Em IL, `string` e `string?` são **o mesmo tipo**; a nulidade de referências não faz parte da assinatura de tipos do runtime. Para que **uma biblioteca compilada** informe seus contratos de nulidade a **outro projeto** (que será analisado separadamente), o compilador grava essa informação em **atributos personalizados** nos metadados do assembly. É uma codificação "por fora" do sistema de tipos.

### Os dois atributos

Definidos em `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/`:

```csharp
// NullableAttribute.cs
[EditorBrowsable(EditorBrowsableState.Never)]
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Property | AttributeTargets.Field
              | AttributeTargets.Event | AttributeTargets.Parameter | AttributeTargets.ReturnValue
              | AttributeTargets.GenericParameter, Inherited = false)]
public sealed class NullableAttribute : Attribute
{
    public readonly byte[] NullableFlags;
    public NullableAttribute(byte value)   { NullableFlags = [value]; }
    public NullableAttribute(byte[] value) { NullableFlags = value; }
}

// NullableContextAttribute.cs
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Struct | AttributeTargets.Method
              | AttributeTargets.Interface | AttributeTargets.Delegate, Inherited = false)]
public sealed class NullableContextAttribute : Attribute
{
    public readonly byte Flag;
    public NullableContextAttribute(byte value) { Flag = value; }
}
```

Comentário nos dois: "Reservado para uso por um compilador para rastrear metadados. Este atributo não deve ser usado por desenvolvedores em código-fonte". Ou seja, é um atributo "pseudo-sintático": o programador nunca o escreve; o compilador o emite.

- **`NullableAttribute`** — anotação **por local**: parâmetro, retorno, campo, propriedade, evento, parâmetro genérico, classe. Aceita um `byte` ou um `byte[]`.
- **`NullableContextAttribute`** — **valor padrão** para um escopo (tipo ou método): "dentro deste escopo, qualquer tipo de referência sem `NullableAttribute` próprio tem este estado". Serve para **compactar**: se a maioria dos membros de uma classe é não-nula, grava-se `NullableContext(1)` na classe e só as exceções recebem `Nullable(2)`.

### A codificação de bytes (confirmada pelo leitor em runtime)

O repositório contém um **consumidor** da codificação: `src/libraries/System.Private.CoreLib/src/System/Reflection/NullabilityInfoContext.cs` (API pública `System.Reflection.NullabilityInfoContext`). Sua função `TranslateByte`:

```csharp
private static NullabilityState TranslateByte(byte b) =>
    b switch
    {
        1 => NullabilityState.NotNull,
        2 => NullabilityState.Nullable,
        _ => NullabilityState.Unknown
    };
```

e `NullabilityState` (em `NullabilityInfo.cs`) tem três valores: `Unknown` ("contexto de nulidade não habilitado — *oblivious*"), `NotNull`, `Nullable`. Portanto: **0 = oblivious, 1 = não-nulo, 2 = anulável**.

### Anotação por posição na árvore de tipos

Um tipo composto tem várias posições: `Dictionary<string, List<string?>?>` tem nulidade para o próprio dicionário, para `string`, para `List<...>` e para o `string?` interno. O compilador grava um `byte[]` em **ordem de travessia pré-ordem do tipo**. O leitor confirma isso em `GetNullabilityInfo(..., ref int index)` (linhas ~338-393):

1. Para um tipo-**valor**: o estado é decidido pelo próprio tipo (`Nullable.GetUnderlyingType(...)` → `Nullable`; senão `NotNull`) e **não consome posição**, exceto se o tipo-valor for genérico (`++index`), porque seus argumentos genéricos ocupam posições;
2. Para um tipo-**referência**: `parser.ParseNullableState(index++, ref state)` lê a posição atual; se não houver array de flags/atributo, cai no `NullableContext` (`GetNullableContext(memberInfo)`), que sobe por `DeclaringType` até achar um (linhas ~33-56);
3. Se for **array**, recorre ao tipo do elemento (consome próxima posição);
4. Se for **genérico**, recorre a cada argumento genérico, em ordem.

O parser `NullableAttributeStateParser` (linhas ~636-669) aceita `byte` (um valor único que vale para todas as posições... no código, o valor único é lido para qualquer `index`) ou uma coleção de `byte` indexada por posição; ausente, retorna `false` e o contexto assume.

Observação: valor único vs array é uma otimização do compilador (um único byte quando todas as posições são iguais). O código do leitor trata `case byte b:` retornando o mesmo estado para qualquer índice, o que confirma o comportamento.

### `NullablePublicOnly`

`NullabilityInfoContext` também respeita o atributo `NullablePublicOnlyAttribute` (referenciado na linha ~307). O comentário no código explica: "se a feature nullablePublicOnly está ativada para um assembly, como no .NET SDK, os atributos de nulidade de membros privados e/ou internos são omitidos; nesse caso a API retorna `NullabilityState.Unknown`". O enum interno `NotAnnotatedStatus { None, Private, Internal }` registra isso por módulo. Motivo: reduzir tamanho de metadados — nulidade de membros não-visíveis não interessa a consumidores.

### Atributos de contrato (outra camada)

Distintos de `Nullable`/`NullableContext`, os atributos de `System.Diagnostics.CodeAnalysis` (`NotNullWhen`, `MaybeNull`, `MemberNotNull`…, ver arquivo 02) são **públicos e escritos pelo desenvolvedor**; eles expressam condições que o tipo `T?` não captura. Os dois mecanismos coexistem: `Nullable*` = o que o compilador **derivou** do `?` no código; `NotNull*` = o que o autor **declarou**.

### Atributos "não chegam ao runtime nativo"

`src/coreclr/tools/aot/ILCompiler.ReadyToRun/Compiler/DependencyAnalysis/ReadyToRun/AttributePresenceFilterNode.cs` (linha ~54) mostra uma decisão pragmática: ao montar a tabela de presença de atributos do ReadyToRun, `System.Runtime.CompilerServices.NullableAttribute` é **sempre excluído** ("há muitos desses, e não fornecem valor útil ao runtime"). Ou seja, o runtime não usa nulidade; ela é só para ferramentas e reflexão.

### Ferramentas que lidam com os atributos

- `src/tools/illink/src/ILLink.RoslynAnalyzer/NullableAttributes.cs`: cópia local dos atributos para o analisador (`netstandard2.0`).
- O projeto de referência (`src/libraries/System.Runtime/ref/System.Runtime.cs`) contém as declarações públicas.
- Configuração: `Nullable` property (arquivo 02). Em `src/libraries/Directory.Build.props` há `enable` para código de produção.

## Por quê

- **Binários independentes**: bibliotecas pré-compiladas precisam transportar contratos; metadados de atributo são o mecanismo extensível já existente do CLR.
- **Retrocompatibilidade**: nenhuma mudança na assinatura de tipos, no JIT, no GC nem no loader.
- **Compactação**: o `NullableContext` evita repetir `Nullable(1)` em cada parâmetro; o `byte` único evita arrays para tipos simples; `NullablePublicOnly` evita anotar membros invisíveis.
- **Reflexão**: a API `NullabilityInfoContext` permite que serializadores, validadores e ORMs leiam nulidade em runtime.

## Trade-offs

| Decisão | Benefício | Custo |
|---|---|---|
| Atributos em vez de tipos | Zero impacto no runtime | Informação perdida se ferramenta ignora atributos; o runtime não impõe nada |
| Codificação posicional `byte[]` | Cobre genéricos aninhados e arrays | Leitor precisa replicar *exatamente* a ordem de travessia (propenso a bugs, ver lógica de `++index` para valor genérico) |
| `NullableContext` por escopo | Metadados menores | Resolução exige subir por `DeclaringType`; leitor mais complexo |
| `NullablePublicOnly` | Menos metadados | Perde-se informação de privados/internos (resultado `Unknown`) |
| Omitir da tabela do R2R | Menos custo no runtime | — |
| Atributo não permitido ao usuário (por convenção) | Evita divergência | Nenhuma imposição técnica além do `EditorBrowsable(Never)` |

## Como aplicar na sua linguagem

1. **Escolha o veículo**: se seu formato de módulo tem metadados extensíveis, use anotações por local como o .NET; se a nulidade fizer parte do tipo, ela entra na assinatura e esta camada some.
2. **Reserve valores explícitos**: `0` = oblivious/desconhecido, `1` = não-nulo, `2` = anulável (e documente). Ter um estado "desconhecido" é essencial para adoção incremental.
3. **Defina uma ordem canônica de travessia do tipo** (pré-ordem, argumentos genéricos da esquerda para a direita, elemento de array antes dos argumentos) e escreva leitores/escritores a partir de **uma só especificação** com testes cruzados.
4. **Ofereça "contexto padrão"** em escopo (tipo/método) para compactar, e uma regra de herança de escopo (método → tipo → tipo externo).
5. **Ofereça otimização "público apenas"** se o seu formato carrega metadados de membros privados.
6. **Forneça uma API de reflexão** equivalente a `NullabilityInfoContext` (`Create(ParameterInfo|PropertyInfo|FieldInfo|EventInfo)`) retornando `{ReadState, WriteState, ElementType, GenericTypeArguments}` — o objeto `NullabilityInfo` do .NET carrega estados de leitura e escrita separados.
7. **Não faça o runtime depender** dessas anotações; exclua-as de tabelas de desempenho (como o R2R faz).
8. **Mantenha atributos "do compilador"** separados dos "de contrato" (`NotNullWhen`…) — papéis e autores distintos.

## Apêndice: exemplo de codificação derivado do algoritmo do leitor

Os bytes abaixo são **deduzidos** aplicando o algoritmo de `GetNullabilityInfo` (confirmado) a um exemplo; não foram extraídos de um assembly compilado, então trate-os como ilustração do algoritmo.

Campo: `Dictionary<string, List<string?>?> mapa;` em contexto `enable`.

Travessia (pré-ordem), cada tipo-referência consome uma posição:

| Posição | Tipo visitado | Estado desejado | Byte |
|---|---|---|---|
| 0 | `Dictionary<,>` (referência) | não-nulo | 1 |
| 1 | `string` (1º arg) | não-nulo | 1 |
| 2 | `List<>` (2º arg) | anulável | 2 |
| 3 | `string` (arg de `List`) | anulável | 2 |

Resultado esperado: `[NullableAttribute(new byte[] { 1, 1, 2, 2 })]`. Se todos fossem não-nulos, o compilador poderia usar `byte` único (ou apenas `NullableContext(1)` no escopo), o que o leitor aceita: `case byte b:` retorna o mesmo estado para qualquer índice, e sem atributo o leitor consulta `GetNullableContext`.

Outro exemplo — tipo-valor genérico: `ValueTuple<string, string?> t;`. `ValueTuple<,>` é tipo-valor genérico: o leitor **não consome posição** para o próprio tipo-valor, mas executa `++index` por ele ser genérico (comportamento confirmado nas linhas ~349-357), e depois lê uma posição para cada argumento: o código nesse ponto deve estar alinhado com o escritor do compilador. Esse tipo de detalhe (quando um tipo consome ou não uma posição) é um ponto delicado de qualquer implementação independente do formato.

### Hierarquia de resolução (como o leitor decide o estado de um tipo-referência)

1. `NullableAttribute` do **próprio membro** (parâmetro/retorno/campo/propriedade), se existir e tiver posição para o índice;
2. senão `NullableContextAttribute` do **membro** (método);
3. senão `NullableContextAttribute` do **tipo** declarante e, subindo, dos tipos externos (loop `memberInfo = memberInfo.DeclaringType` em `GetNullableContext`);
4. senão `Unknown` (oblivious).

Há também cache: `_context` (`Dictionary<MemberInfo, NullabilityState>`) evita reler atributos.

### Para onde isso aponta no design

| Necessidade | Solução vista no repo |
|---|---|
| Transportar contratos entre assemblies | Atributos de metadados `Nullable`, `NullableContext` |
| Reduzir tamanho | Byte único, contexto por escopo, `NullablePublicOnly` |
| Reflexão | `NullabilityInfoContext` (API pública) |
| Desempenho do runtime | Nada: `AttributePresenceFilterNode` nem inclui `NullableAttribute` |
| Compat. com compiladores antigos | Atributos são só tipos comuns; ignorados por quem não entende |
| Biblioteca `netstandard2.0` | Cópia local dos atributos (`internal`) |

### Checklist para um formato de metadados de nulidade

1. Defina a semântica dos valores (`0/1/2`) e os reserve para sempre.
2. Defina a ordem de travessia com exemplos (como o da tabela acima).
3. Defina a regra de contexto (escopo padrão) e a regra de override.
4. Publique um leitor de referência e testes cruzados escritor↔leitor.
5. Defina compatibilidade futura: valores desconhecidos devem ser lidos como `Unknown` (o leitor faz `_ => Unknown`).

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/NullableAttribute.cs`, `NullableContextAttribute.cs`
- `src/libraries/System.Private.CoreLib/src/System/Reflection/NullabilityInfoContext.cs`, `NullabilityInfo.cs`
- `src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/NullableAttributes.cs`
- `src/coreclr/tools/aot/ILCompiler.ReadyToRun/Compiler/DependencyAnalysis/ReadyToRun/AttributePresenceFilterNode.cs`
- `src/tools/illink/src/ILLink.RoslynAnalyzer/NullableAttributes.cs`
- `src/libraries/Directory.Build.props` (propriedade `Nullable`)
- Não presente no repo: especificação de nullable metadata do compilador (docs do Roslyn / C# `csharp-8.0/nullable-reference-types-specification`).
