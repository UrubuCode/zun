# Source generators como análise + geração

> Source generators são um recurso do compilador (Roslyn); a infraestrutura em si não está no `dotnet/runtime`. O repositório contém, porém, **muitos geradores reais** e documentos de diretriz e de design. Tudo abaixo é baseado nesses fontes. Detalhes da API Roslyn não mostrados nos arquivos são conhecimento geral e marcados.

## Mecanismo no .NET

### Ideia: análise do código-fonte → emissão de código-fonte

Um gerador de código-fonte roda **durante a compilação**, recebe uma visão semântica do programa (símbolos, atributos, sintaxe) e **adiciona novos arquivos-fonte** à compilação. Ele é, ao mesmo tempo:

1. um **analisador** (inspeciona declarações marcadas, valida entrada, reporta diagnósticos);
2. um **gerador** (emite C# determinístico).

Isso substitui trabalho que antes era feito em *runtime* com reflexão ou IL dinâmico. Motivação textual de `docs/design/features/source-generator-pinvokes.md`: o marshalling nativo do CLR gera "IL stubs" em runtime; consequências listadas: correções dependem de atualizar o runtime inteiro; `ICustomMarshaler` tem custo substancial; difícil depurar; e "o IL Linker também tem dificuldade com código gerado em runtime, pois não consegue entender todos os tipos potencialmente usados sem ver o que é gerado". Objetivos: "evoluir P/Invoke independentemente do runtime" e "alto desempenho: sem reflexão em runtime, compatível com AOT".

### Geradores no repositório (confirmados)

| Gerador | Local | Entrada | Saída |
|---|---|---|---|
| `JsonSourceGenerator` | `src/libraries/System.Text.Json/gen/` | `[JsonSerializable(typeof(T))]` | Metadados/serializadores para `T`, sem reflexão |
| `RegexGenerator` | `src/libraries/System.Text.RegularExpressions/gen/` | `[GeneratedRegex("...")]` em método/propriedade `partial` | Classe `Regex` derivada com código específico do padrão |
| `LoggerMessageGenerator` | `src/libraries/Microsoft.Extensions.Logging.Abstractions/gen/` | `[LoggerMessage]` | Métodos de log fortemente tipados |
| `LibraryImportGenerator` | `src/libraries/System.Runtime.InteropServices/gen/LibraryImportGenerator/` | `[LibraryImport]` | Stub de marshalling + `[DllImport]` interno |
| `ComInterfaceGenerator` | `.../gen/ComInterfaceGenerator/` | `[GeneratedComInterface]` | Wrappers COM via `ComWrappers` |
| `JSImportGenerator` | `src/libraries/System.Runtime.InteropServices.JavaScript/gen/` | `[JSImport]/[JSExport]` | Stubs de interop JS |
| Binder de configuração | `src/libraries/Microsoft.Extensions.Configuration.Binder/gen/` | chamadas `Bind/Get` | Binding estático (com supressor de `IL3050`) |
| Options | `src/libraries/Microsoft.Extensions.Options/gen/` | validadores | Código de validação |
| CoreLib interno | `src/libraries/System.Private.CoreLib/gen/` | `EventSourceGenerator`, `NativeRuntimeEventSourceGenerator`, `ProductVersionInfoGenerator` | EventSource, versão |

### O padrão incremental (confirmado em `RegexGenerator.cs`, `LoggerMessageGenerator.Roslyn4.0.cs`, `JsonSourceGenerator.Roslyn4.0.cs`)

Todos usam `[Generator]` e implementam `IIncrementalGenerator`. O pipeline é:

```csharp
context.SyntaxProvider
    .ForAttributeWithMetadataName(NomeDoAtributo, predicado, transform)   // 1. encontrar
    .Where(static m => m is not null)
    .Select(...)                                                          // 2. analisar/modelar
    .Select(...)                                                          // 3. gerar texto
    ...
context.RegisterSourceOutput(modelo, (spc, items) => EmitSource(items, spc)); // 4. adicionar fontes
context.RegisterSourceOutput(diagnosticos, EmitDiagnostics);                  // 5. reportar diagnósticos
```

O comentário em `RegexGenerator.cs` explica o contrato de **cache incremental**:

> O predicado será executado uma vez para cada nó com atributo no arquivo que está sendo modificado. O transform será executado uma vez para cada nó com atributo na compilação. Portanto, ambos devem fazer o mínimo de trabalho e sair. Isto também deve ter extraído tudo do alvo necessário para toda análise subsequente e retornar um objeto **comparável por valor** que **não referencie nada da compilação**: queremos garantir que quaisquer resultados em cache bem-sucedidos sejam idempotentes para a entrada, de forma que não disparem trabalho a jusante se não houver mudanças.

Outro detalhe: "A localização é passada separadamente dos records para que não participe da igualdade do record — isso permite que o pipeline incremental armazene resultados em cache por valor."

Para viabilizar isso, o repo oferece utilitários em `src/libraries/Common/src/SourceGenerators/`: `ImmutableEquatableArray.cs` ("lista imutável que implementa igualdade de sequência", exige `T : IEquatable<T>`), `TypeRef.cs`, `DeclarationHeader.cs`, `ContainingTypeUtilities.cs`, `SourceWriter.cs`, `CSharpSyntaxUtilities.cs`, `TypeModelHelper.cs`. A razão: coleções padrão (`ImmutableArray<T>`) têm igualdade por referência e invalidariam o cache.

### Separação Parser / Model / Emitter

- `JsonSourceGenerator.Parser.cs` e `JsonSourceGenerator.Emitter.cs` + pasta `Model/` (JSON); `RegexGenerator.Parser.cs` e `RegexGenerator.Emitter.cs`; `LoggerMessageGenerator.Parser.cs`/`.Emitter.cs`. O **Parser** lê símbolos Roslyn e produz um **modelo de valores** (records comparáveis); o **Emitter** só lê o modelo e escreve texto. `docs/design/libraries/LibraryImportGenerator/Pipeline.md` descreve: "A análise de entrada extrai nomes de tipos, atributos, declarações contenedoras e informações de marshalling para **modelos de valor**. A emissão opera nesses modelos, **não em árvores de sintaxe**." Também: "APIs de sintaxe do Roslyn permanecem apropriadas para inspecionar declarações de entrada e implementar code fixes; **não devem ser usadas para construir, parsear ou normalizar a saída gerada**".
- Passos do `LibraryImportGenerator` (Pipeline.md): (1) processar símbolos e metadados; (2) determinar os geradores de marshalling por parâmetro/retorno; (3) gerar o código do stub; (4) gerar o P/Invoke correspondente; (5) adicionar o código à compilação. Marshallers escrevem "estágios" em um writer; o gerador compõe estágios preservando escopos de pinning, tratamento de exceção e ordem de limpeza.

### Diretrizes do repo (`docs/coding-guidelines/source-generator-guidelines.md`)

DOs: gerar código "como se um desenvolvedor o tivesse escrito à mão"; **emitir strings, não usar a Syntax API**, para desempenho; indentação consistente; usar `IIncrementalGenerator`; `EnforceExtendedAnalyzerRules`; `IsRoslynComponent`; **emitir diagnósticos de um analisador separado** (podem estar no mesmo assembly); fazer cache de resultados intermediários; considerar impacto no tempo de build; projetos separados para testar o gerador e o código gerado; usar o Roslyn Testing SDK.
DON'Ts: usar a Syntax API para emitir código; operações caras na geração; introduzir dependências de runtime não referenciadas pelo projeto; emitir código que dispare warnings em uso normal; **emitir diagnósticos do próprio gerador**.

(Observação: o `RegexGenerator.cs` ainda tem `RegisterSourceOutput(diagnosticResults, ...)` na linha ~325 — a prática do repo é mista; a diretriz é a regra declarada.)

### Cabeçalho de arquivos gerados

`RegexGenerator.cs` define `s_headers` incluindo `// <auto-generated/>`, `#nullable enable`, e em release `#pragma warning disable` (em debug, apenas CS0162 "código inalcançável", CS0164, CS0219). O código gerado é anotado com `[GeneratedCode(...)]` com nome e versão do assembly do gerador. Nota: `CompilationData(bool AllowUnsafe, bool CheckOverflow, LanguageVersion LanguageVersion)` — o gerador **lê opções da compilação** (unsafe permitido, checagem de overflow, versão da linguagem) para decidir o que emitir, e passa `CheckOverflow` para `EmitRegexDerivedTypeRunnerFactory`. É um bom exemplo de geração sensível ao contexto (conecta com o arquivo 08).

### Degradação graciosa

No `RegexGenerator`, se o padrão não é suportado por geração completa (`SupportsCodeGeneration`), emite diagnóstico `LimitedSourceGeneration` e gera uma implementação limitada que apenas cacheia `new Regex(...)`. Se o padrão é inválido, retorna `Diagnostic.Create(DiagnosticDescriptors.InvalidRegexArguments, ...)`. Padrão: **análise → decisão → fallback com aviso**, nunca falha silenciosa.

### Relação com trimming/AOT

Geradores são a resposta do .NET aos limites da reflexão (arquivos 04, 06, 10). `reflection-flow.md` afirma: serialização e injeção de dependência "seriam melhor resolvidos por source generators". O supressor `ConfigurationBindingGenerator.Suppressor.cs` (`SYSLIBSUPPRESS0003`, `IL3050`) suprime o aviso de `RequiresDynamicCode` na chamada de binding interceptada por uma variante estática gerada.

### Análise que acompanha o gerador

Cada gerador relevante vem com analisadores e code fixes irmãos: `UpgradeToGeneratedRegexAnalyzer.cs` + `UpgradeToGeneratedRegexCodeFixer.cs` (sugere migrar `new Regex("...")` para `[GeneratedRegex]`), `ConvertToLibraryImportAnalyzer.cs` (migra `DllImport`), `ComClassGeneratorDiagnosticsAnalyzer.cs`, `AddGeneratedComClassAnalyzer.cs`, etc. (arquivo 05). Os geradores de interop compartilham `Microsoft.Interop.SourceGeneration` (`src/libraries/System.Runtime.InteropServices/gen/Microsoft.Interop.SourceGeneration`).

## Por quê

- **Mover trabalho de runtime para build**: código gerado em compilação é AOT-friendly, depurável, sem reflexão e sem JIT.
- **Evolução independente do runtime**: o marshaller de `LibraryImport` evolui sem atualizar o CLR.
- **Estático = analisável**: o código gerado é visível ao trimmer e ao compilador AOT.
- **Experiência do IDE**: pipeline incremental mantém a edição responsiva (só recomputa o que mudou).
- **Combinação análise + geração**: usar o mesmo modelo para validar (diagnósticos) e emitir.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Geração em build | AOT/trim-friendly, depuração, desempenho | Tempo de build; complexidade do gerador; geradores só veem o que o compilador mostra |
| Pipeline incremental com modelos de valor | Cache eficiente no IDE | Disciplina rígida: nada de referências à compilação nos modelos (`ImmutableEquatableArray`, `TypeRef`) |
| Emitir strings, não árvores de sintaxe | Rápido, legível | Risco de código inválido; precisa de escape correto (`CodeWriterHelpers.StringLiteral`) |
| Diagnósticos em analisador separado (diretriz) | Reuso, responsividade | Duplica parsing da entrada; o repo ainda mistura |
| `partial` obrigatório | Integra com código do usuário | Exige que todos os tipos contenedores sejam `partial` (regra explícita do JSON generator) |
| Fallback com aviso (regex limitado) | Nada quebra | Desempenho abaixo do esperado sem o usuário notar se ignorar o aviso |
| Geradores por versão de Roslyn (`Roslyn3.11`, `Roslyn4.0`, `Roslyn4.4` csproj) | Compat | Múltiplos alvos para manter |

## Como aplicar na sua linguagem

1. **Ofereça um mecanismo de metaprogramação em tempo de compilação** que (a) enxergue símbolos e atributos, (b) adicione arquivos-fonte, (c) seja determinístico. Se sua linguagem é compilada para IR, o ponto de extensão natural é *antes* da análise semântica final.
2. **Torne-o incremental desde o dia 1**: defina um pipeline de passos puros `entrada → modelo` com caching por igualdade de valor, e proíba referências à AST nos modelos.
3. **Separe Parser (AST → modelo) e Emitter (modelo → texto)**. Teste o Emitter com modelos escritos à mão.
4. **Emita texto**, com um `IndentedTextWriter` e escaping central; dê uma API para literais de string.
5. **Marque o código gerado** (`<auto-generated/>`, atributo `GeneratedCode`) e dê ao compilador regras para suprimir avisos nele e (decisão sua) analisá-lo.
6. **Dê aos geradores acesso às opções do compilador** (checagem de overflow, unsafe, versão da linguagem), mas faça com que façam parte do modelo para que o cache seja invalidado quando mudarem.
7. **Acompanhe cada gerador de um analisador e um code fix** que ajudem a migrar de código reflexivo e a corrigir uso incorreto.
8. **Defina o contrato de falha**: diagnóstico + fallback seguro (como `LimitedSourceGeneration`), sem exceções que derrubem a compilação.
9. **Evite dependência cíclica**: o gerador não deve exigir que o código gerado esteja presente para compilar a própria entrada (`partial` + declarações vazias ajudam).
10. **Teste em dois projetos**: um para o gerador (diagnósticos e texto) e outro que compila e executa o código gerado, como pede a diretriz do repo.

## Apêndice: esqueleto, comparação e exemplo

### Esqueleto de um gerador incremental (conhecimento geral sobre a API; estrutura igual à dos geradores do repo)

```csharp
[Generator(LanguageNames.CSharp)]
public sealed class MeuGerador : IIncrementalGenerator
{
    public void Initialize(IncrementalGeneratorInitializationContext context)
    {
        var modelos = context.SyntaxProvider.ForAttributeWithMetadataName(
                "Meu.Namespace.MeuAtributo",
                static (node, _) => node is MethodDeclarationSyntax,           // predicado barato
                static (ctx, _)  => Parser.Extrair(ctx))                      // retorna modelo comparável por valor
            .Where(static m => m is not null);

        context.RegisterSourceOutput(modelos, static (spc, modelo) =>
            spc.AddSource($"{modelo.Nome}.g.cs", Emitter.Emitir(modelo)));   // texto, não árvores de sintaxe
    }
}
```

### Comparação dos geradores do repo

| Gerador | O que substitui em runtime | Modelo de entrada | Fallback |
|---|---|---|---|
| Regex | Interpretador/compilação de `Regex` em runtime (Reflection.Emit) | `RegexTree` + `AnalysisResults` | Implementação limitada que cacheia `new Regex(...)` + diagnóstico |
| JSON | Reflexão sobre tipos serializáveis | Spec do contexto (`ContextGenerationSpec`, `ClassType`, `CollectionType`…) | Diagnósticos de tipos não suportados |
| LibraryImport | IL stubs de marshalling do CLR | `TypePositionInfo` + `MarshallingInfo` | Diagnósticos de marshalling não suportado |
| Logging | Formatação/alocação em cada chamada | Modelo de métodos `[LoggerMessage]` | Diagnósticos de assinatura inválida |
| Configuration binder | Reflexão ao ligar config a objetos | Chamadas interceptadas | Supressão IL3050 |

(Os nomes `ContextGenerationSpec`, `ClassType` e `CollectionType` foram vistos em `src/libraries/System.Text.Json/gen/Model/`.)

### Fluxo ponta a ponta com a análise estática do arquivo 04/06

1. O desenvolvedor escreve `[GeneratedRegex("^\\d+$")] static partial Regex Digits();`.
2. O gerador valida o padrão (análise) e emite um tipo derivado de `Regex` com código específico (geração).
3. O resultado é código C# normal: sem reflexão, sem `RequiresDynamicCode`. O trimmer e o AOT o analisam como qualquer outro; nada precisa de anotação extra.
4. Se o dev usasse `new Regex(pattern)`, o analisador `UpgradeToGeneratedRegexAnalyzer` sugere a migração (arquivo 05).

### Regras de projeto resumidas

- Entrada: atributos + `partial`. Sem comportamentos mágicos escondidos.
- Saída: código legível, estável, com cabeçalho `<auto-generated/>` e `GeneratedCode`.
- Cache: modelos de valor, `ImmutableEquatableArray`, localização fora da igualdade.
- Falha: diagnóstico + fallback, nunca exceção.
- Teste: duas camadas (gerador e código gerado).

### Quando **não** usar geração

- Quando a decisão depende de dados só conhecidos em runtime (carregamento dinâmico de plug-ins): aí use anotações (`DynamicallyAccessedMembers`) e aceite os avisos.
- Quando o compilador não expõe a informação necessária ao gerador (geradores só enxergam o que o compilador mostra, não o código de outros geradores no mesmo passo, conhecimento geral).
- Quando o custo no build supera o benefício (as diretrizes do repo pedem considerar o impacto no tempo de build).

## Referências

- `docs/coding-guidelines/source-generator-guidelines.md`
- `docs/design/features/source-generator-pinvokes.md`, `source-generator-com.md`
- `docs/design/libraries/LibraryImportGenerator/Pipeline.md` (e `Compatibility.md`)
- `src/libraries/System.Text.RegularExpressions/gen/RegexGenerator.cs`, `RegexGenerator.Parser.cs`, `RegexGenerator.Emitter.cs`, `UpgradeToGeneratedRegexAnalyzer.cs`
- `src/libraries/System.Text.Json/gen/JsonSourceGenerator.Roslyn4.0.cs`, `JsonSourceGenerator.Parser.cs`, `JsonSourceGenerator.Emitter.cs`, `Model/`
- `src/libraries/Microsoft.Extensions.Logging.Abstractions/gen/LoggerMessageGenerator.Roslyn4.0.cs`
- `src/libraries/Common/src/SourceGenerators/ImmutableEquatableArray.cs`, `SourceWriter.cs`, `TypeRef.cs`, `DeclarationHeader.cs`
- `src/libraries/System.Runtime.InteropServices/gen/` (`LibraryImportGenerator`, `ComInterfaceGenerator`, `Microsoft.Interop.SourceGeneration`)
- `src/libraries/Microsoft.Extensions.Configuration.Binder/gen/ConfigurationBindingGenerator.Suppressor.cs`; `docs/project/list-of-diagnostics.md`
- `docs/design/tools/illink/reflection-flow.md`
