# Analisadores Roslyn — conceito e diagnostics

> O framework `Microsoft.CodeAnalysis` (Roslyn) em si não está em `dotnet/runtime`. O repositório contém, porém, **muitos analisadores, supressores, code fixes e geradores** escritos sobre ele. Este documento descreve o que se confirma nesses fontes e extrai o modelo de design. Detalhes internos da API Roslyn são mencionados como "conhecimento geral".

## Mecanismo no .NET

### O que é um analisador

Um analisador Roslyn é uma classe derivada de `DiagnosticAnalyzer`, marcada com `[DiagnosticAnalyzer(LanguageNames.CSharp)]`, que expõe:

1. `SupportedDiagnostics` — a lista de `DiagnosticDescriptor` (id, título, mensagem, categoria, severidade, habilitado por padrão, link de ajuda);
2. `Initialize(AnalysisContext)` — onde registra *callbacks* (por símbolo, por nó de sintaxe, por operação, por compilação, por bloco de código).

O compilador chama os callbacks durante a compilação e coleta os `Diagnostic` reportados. Os analisadores rodam no IDE e no build.

### Exemplos reais no repositório

Lista (confirmada por busca de `DiagnosticAnalyzer(LanguageNames`) fora de testes:

| Grupo | Arquivos |
|---|---|
| Trimming/AOT/Single-file | `src/tools/illink/src/ILLink.RoslynAnalyzer/RequiresUnreferencedCodeAnalyzer.cs`, `RequiresDynamicCodeAnalyzer.cs`, `RequiresAssemblyFilesAnalyzer.cs`, `DynamicallyAccessedMembersAnalyzer.cs`, `COMAnalyzer.cs` |
| Segurança de `unsafe` | `PointerSignatureRequiresUnsafeAnalyzer.cs`, `LibraryImportRequiresExplicitSafetyAnalyzer.cs`, `UnsafeMemberMissingSafetyDocumentationAnalyzer.cs` |
| Interop (`LibraryImport`) | `src/libraries/System.Runtime.InteropServices/gen/LibraryImportGenerator/Analyzers/ConvertToLibraryImportAnalyzer.cs`, `CustomMarshallerAttributeAnalyzer.cs`, `NativeMarshallingAttributeAnalyzer.cs`, `LibraryImportDiagnosticsAnalyzer.cs` |
| Interop (COM) | `.../gen/ComInterfaceGenerator/Analyzers/*.cs` (ex.: `AddGeneratedComClassAnalyzer`, `GeneratedComInterfaceAttributeAnalyzer`, `RuntimeComApiUsageWithSourceGeneratedComAnalyzer`) |
| JS interop | `src/libraries/System.Runtime.InteropServices.JavaScript/gen/JSImportGenerator/Analyzers/JSImportDiagnosticsAnalyzer.cs`, `JSExportDiagnosticsAnalyzer.cs` |
| Regex | `src/libraries/System.Text.RegularExpressions/gen/UpgradeToGeneratedRegexAnalyzer.cs` (+ `UpgradeToGeneratedRegexCodeFixer.cs`) |
| Regras internas do runtime | `src/libraries/System.Private.CoreLib/gen/IntrinsicsInSystemPrivateCoreLibAnalyzer.cs` |
| Supressor | `src/libraries/Microsoft.Extensions.Configuration.Binder/gen/ConfigurationBindingGenerator.Suppressor.cs`, `ShapeBreakingDiagnosticSuppressor.cs` |

Portanto o repo mostra três "papéis": **analisadores que reportam problemas** (ILLink), **analisadores que sugerem migração** (`ConvertToLibraryImport`, `UpgradeToGeneratedRegex`) e **supressores** que retiram avisos de outros analisadores quando o contexto os torna inválidos (ex.: o supressor de `IL3050` do gerador de binding de configuração, listado em `docs/project/list-of-diagnostics.md` como `SYSLIBSUPPRESS0003`).

### Anatomia: o analisador `Requires*`

`RequiresAnalyzerBase.cs` (`src/tools/illink/src/ILLink.RoslynAnalyzer/`) é uma classe base abstrata com os "pontos de variação" de cada atributo:

```csharp
private protected abstract string RequiresAttributeName { get; }
internal abstract string RequiresAttributeFullyQualifiedName { get; }
private protected abstract DiagnosticTargets AnalyzerDiagnosticTargets { get; }
private protected abstract DiagnosticDescriptor RequiresDiagnosticRule { get; }
private protected abstract DiagnosticId RequiresDiagnosticId { get; }
private protected abstract DiagnosticDescriptor RequiresAttributeMismatch { get; }
private protected abstract DiagnosticDescriptor RequiresOnStaticCtor { get; }
private protected abstract DiagnosticDescriptor RequiresOnEntryPoint { get; }
```

`RequiresUnreferencedCodeAnalyzer` preenche isso com `DiagnosticId.RequiresUnreferencedCode`, e o mesmo esqueleto serve a `RequiresDynamicCodeAnalyzer` e `RequiresAssemblyFilesAnalyzer`. Cada um decide **se está habilitado** lendo uma propriedade MSBuild exposta ao compilador:

```csharp
internal override bool IsAnalyzerEnabled(AnalyzerOptions options) =>
    options.IsMSBuildPropertyValueTrue(MSBuildPropertyOptionNames.EnableTrimAnalyzer);
```

As propriedades `EnableTrimAnalyzer`, `EnableAotAnalyzer`, `EnableSingleFileAnalyzer`, `VerifyReferenceTrimCompatibility`, `VerifyReferenceAotCompatibility`, `IncludeAllContentForSelfExtract` são declaradas como `CompilerVisibleProperty` em `src/tools/illink/src/ILLink.RoslynAnalyzer/build/Microsoft.NET.ILLink.Analyzers.props` (confirmado) — esse é o mecanismo para o analisador enxergar configuração de projeto.

No `Initialize` (confirmado, `RequiresAnalyzerBase.cs` ~linha 70):

- `context.ConfigureGeneratedCodeAnalysis(GeneratedCodeAnalysisFlags.Analyze | GeneratedCodeAnalysisFlags.ReportDiagnostics)` — **analisa código gerado também** (necessário porque source generators produzem chamadas que podem violar regras);
- `context.EnableConcurrentExecution()` (exceto com depurador anexado);
- `RegisterCompilationStartAction`, `RegisterSymbolAction`, `RegisterSyntaxNodeAction`, `RegisterCompilationAction`, com pontos de extensão (`ExtraSyntaxNodeActions`, `ExtraSymbolActions`, `ExtraCompilationActions`).

### Descritores e IDs compartilhados

`DiagnosticDescriptors.GetDiagnosticDescriptor(DiagnosticId)` (arquivo `DiagnosticDescriptors.cs`) constrói **todos** os descritores a partir de `ILLink.Shared/DiagnosticId.cs` + `SharedStrings.resx`: severidade `Warning`, `isEnabledByDefault: true`, categoria e helplink derivados do id. Isso garante que o mesmo código `IL2026` seja reportado pelo trimmer em build, pelo AOT e pelo analisador no IDE, com mesmo texto. Os IDs relevantes (de `DiagnosticId.cs`): `RequiresUnreferencedCode = 2026`, `RequiresUnreferencedCodeAttributeMismatch = 2046`, `RequiresAssemblyFiles = 3002`, `RequiresDynamicCode = 3050`, e muitos `DynamicallyAccessedMembers*` (21xx). Faixas: `IL2xxx` trimming, `IL3xxx` single-file/AOT.

### Dataflow dentro do analisador

Para os avisos de `DynamicallyAccessedMembers`, o analisador reimplementa a análise de fluxo em cima do **CFG da IOperation** de Roslyn: `ILLink.RoslynAnalyzer/DataFlow/` contém `LocalDataFlowAnalysis.cs`, `LocalDataFlowVisitor.cs`, `LocalStateLattice.cs`, `ControlFlowGraphProxy.cs`, `FeatureChecksVisitor.cs`, e `TrimAnalysis/TrimAnalysisVisitor.cs`, `HandleCallAction.cs`, `FlowAnnotations.cs`. `HandleCallAction`, `RequireDynamicallyAccessedMembersAction` e `ValueWithDynamicallyAccessedMembers` são **compartilhados** com o trimmer via `ILLink.Shared/TrimAnalysis/` — as mesmas regras, duas "frentes" (IL e C#).

### Code fixes

`src/tools/illink/src/ILLink.CodeFix/` contém `RequiresUnreferencedCodeCodeFixProvider.cs`, `RequiresDynamicCodeCodeFixProvider.cs`, `RequiresAssemblyFilesCodeFixProvider.cs`, `DynamicallyAccessedMembersCodeFixProvider.cs` (com `TrimAnalysis/DAM-code-fix.md` documentando o comportamento), `UnconditionalSuppressMessageCodeFixProvider.cs`, além de fixes de `unsafe` (`AddUnsafeTo*`). O analisador diz "o que está errado" e o code fix oferece a edição (adicionar atributo, suprimir).

### Testes

`src/tools/illink/test/ILLink.RoslynAnalyzer.Tests/` tem testes como `DynamicallyAccessedMembersAnalyzerTests.cs`, `DataFlowTests.cs`, `GenericsTests.cs`, `ReferenceAotCompatibilityTests.cs`, `*CodeFixTests.cs`. `ILLink.RoslynAnalyzer.Tests.Generator` gera testes a partir dos casos do trimmer (`Mono.Linker.Tests.Cases`) — um método para **manter analisador (C#) e trimmer (IL) consistentes**.

### Habilitação em bibliotecas do repo

`src/libraries/Directory.Build.props` (linhas ~66-69): `IsAotCompatible` padrão `true` em projetos de código-fonte e de referência; `EnableTrimAnalyzer` e `EnableAotAnalyzer` ligados em projetos de teste; `eng/liveILLink.targets` e `eng/illink.targets` conectam os analisadores recém-compilados ao build do próprio repositório.

## Por quê

- **Feedback no momento da escrita**: reflexão e `RequiresUnreferencedCode` só são detectáveis pelo trimmer no publish; o analisador antecipa o aviso para o editor/CI.
- **Uma regra, um código, muitas ferramentas**: IDs estáveis (`IL2026`) permitem documentação (`docs/tools/illink/error-codes.md`), supressão (`UnconditionalSuppressMessage`) e `SuppressMessage` consistentes.
- **Extensibilidade da linguagem sem mudar o compilador**: o runtime acrescenta regras de uso de API (migrar `DllImport` para `LibraryImport`, usar `GeneratedRegex`) via analisadores + code fixes.
- **Análise de código gerado**: `GeneratedCodeAnalysisFlags.Analyze` evita que geradores sejam "buraco" na análise.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Analisador fora do compilador | Evolui com o framework; opt-in por projeto | Precisa reimplementar análise (`ILLink.RoslynAnalyzer/DataFlow` ≈ duplica a do trimmer); pode divergir |
| Severidade `Warning` por padrão | Não quebra builds | Fácil de ignorar |
| `netstandard2.0` para o analisador | Roda em qualquer versão de compilador | Sem BCL moderna (cópias de atributos; `NullableAttributes.cs`) |
| Gating por propriedade MSBuild | Zero ruído para quem não otimiza | Dependência de infraestrutura de build (`CompilerVisibleProperty`) |
| Ativar em código gerado | Cobertura | Falsos positivos em código que o usuário não controla → supressores |
| Strings/IDs compartilhados | Consistência | Acoplamento entre produtos (`ILLink.Shared` compilado em vários) |

## Como aplicar na sua linguagem

1. **Exponha uma API de plug-ins de análise com descritores declarativos** (id, título, mensagem, categoria, severidade padrão, helplink). Permita que bibliotecas distribuam regras junto do pacote (modelo `Analyzer` item do NuGet).
2. **Dê IDs estáveis e uma tabela de docs por ID desde a v1**; use faixas por tema (`IL2xxx`, `IL3xxx`).
3. **Separe "regra" de "motor"**: coloque o modelo de valores/ações em uma biblioteca compartilhada (como `ILLink.Shared`) para ser usada tanto pela análise de fonte quanto pela de binário/IR.
4. **Exponha propriedades de build ao analisador** (equivalente a `CompilerVisibleProperty`) para ligar/desligar regras sem editar código.
5. **Ofereça code fixes** para os 3-4 erros mais comuns; é o que torna a regra adotável.
6. **Defina a política para código gerado** (analisar? suprimir?). O .NET escolheu analisar e dar supressores explícitos.
7. **Teste com cenários mínimos e gere testes a partir da suíte da outra ferramenta** (como `ILLink.RoslynAnalyzer.Tests.Generator`) para garantir paridade fonte/binário.
8. **Planeje concorrência** (o analisador deve ser thread-safe; `EnableConcurrentExecution`).

## Apêndice: faixas de IDs, esqueleto de analisador e política de qualidade

### Faixas de `DiagnosticId` (de `ILLink.Shared/DiagnosticId.cs`)

| Faixa | Seção do enum | Exemplos confirmados |
|---|---|---|
| 1xxx | Erros do linker/configuração | `XmlFeatureDoesNotSpecifyFeatureValue = 1001`, `RootAssemblyDoesNotHaveEntryPoint = 1034`, `InvalidAssemblyName = 1036` |
| 2xxx | "Trimming diagnostic ids" | `TypeHasNoFieldsToPreserve = 2001`, `RequiresUnreferencedCode = 2026`, `RequiresUnreferencedCodeAttributeMismatch = 2046`, `DynamicallyAccessedMembersOnTypeReferencesMemberWithRequiresUnreferencedCode = 2112`, `RequiresUnreferencedCodeOnStaticConstructor = 2116`, `DynamicallyAccessedMembersIsNotAllowedOnExtensionProperties = 2127` |
| 3000-3005 | Single-file | `AvoidAssemblyLocationInSingleFile = 3000`, `RequiresAssemblyFiles = 3002` |
| 3050-3058 | Dynamic code / AOT | `RequiresDynamicCode = 3050`, `ReferenceNotMarkedIsAotCompatible = 3058` |
| 4000-4001 | Feature guards | `ReturnValueDoesNotMatchFeatureGuards`, `InvalidFeatureGuard` |
| 5005-5006 | Migração de segurança de memória (`unsafe`) | `UnsafeMemberMissingSafetyDocumentation`, `PointerSignatureRequiresUnsafe` |

Existem dois "sentinelas" no enum, `_EndTrimAnalysisWarningsSentinel` e `_EndAotAnalysisWarningsSentinel`, usados para **classificar** um ID por faixa (ex.: `>= 2109 and < _EndTrimAnalysisWarningsSentinel => MessageSubCategory.TrimAnalysis`, linha ~248). Boa técnica: o ID numérico codifica a categoria, evitando tabelas paralelas.

### Esqueleto mínimo de um analisador (conhecimento geral sobre a API Roslyn)

```csharp
[DiagnosticAnalyzer(LanguageNames.CSharp)]
public sealed class MeuAnalisador : DiagnosticAnalyzer
{
    static readonly DiagnosticDescriptor Regra = new("XY0001", "Título", "Mensagem {0}",
        "Categoria", DiagnosticSeverity.Warning, isEnabledByDefault: true);

    public override ImmutableArray<DiagnosticDescriptor> SupportedDiagnostics => ImmutableArray.Create(Regra);

    public override void Initialize(AnalysisContext ctx)
    {
        ctx.ConfigureGeneratedCodeAnalysis(GeneratedCodeAnalysisFlags.None);
        ctx.EnableConcurrentExecution();
        ctx.RegisterSyntaxNodeAction(Analisar, SyntaxKind.InvocationExpression);
    }
    static void Analisar(SyntaxNodeAnalysisContext c) { /* checar e c.ReportDiagnostic(...) */ }
}
```

O analisador real do ILLink segue exatamente esta forma, mas com: descritores gerados de uma fonte compartilhada (`DiagnosticDescriptors.GetDiagnosticDescriptor(DiagnosticId)`), `GeneratedCodeAnalysisFlags.Analyze | ReportDiagnostics`, e uma base abstrata para as três variantes `Requires*`.

### Política de qualidade nos analisadores do repo (da `source-generator-guidelines.md`)

- `EnforceExtendedAnalyzerRules=true`: o compilador proíbe APIs banidas em analisadores/geradores (ex.: acesso a arquivos, relógio), garantindo determinismo.
- `IsRoslynComponent=true`: habilita depuração no Visual Studio.
- `RS2008` (releases tracking) desabilitado: "tratado de forma diferente no repositório runtime".
- Diagnósticos devem sair de um **analisador separado**, não do gerador (arquivo 12).

### Ciclo completo "regra → experiência"

1. Defina `DiagnosticId` + string em `SharedStrings.resx`.
2. Implemente o analisador (reporta).
3. Implemente o code fix (`ILLink.CodeFix/`), recebendo dados via `properties` do `Diagnostic`.
4. Documente em `docs/tools/illink/error-codes.md`, com exemplo mínimo de código.
5. Adicione testes em `ILLink.RoslynAnalyzer.Tests` e, se aplicável, um caso em `Mono.Linker.Tests.Cases` (o gerador de testes cria o espelho).
6. Habilite via propriedade MSBuild (`EnableTrimAnalyzer` etc.).

## Referências

- `src/tools/illink/src/ILLink.RoslynAnalyzer/RequiresAnalyzerBase.cs`, `RequiresUnreferencedCodeAnalyzer.cs`, `RequiresDynamicCodeAnalyzer.cs`, `RequiresAssemblyFilesAnalyzer.cs`, `DynamicallyAccessedMembersAnalyzer.cs`, `DiagnosticDescriptors.cs`
- `src/tools/illink/src/ILLink.RoslynAnalyzer/build/Microsoft.NET.ILLink.Analyzers.props`
- `src/tools/illink/src/ILLink.RoslynAnalyzer/DataFlow/`, `TrimAnalysis/`
- `src/tools/illink/src/ILLink.Shared/DiagnosticId.cs`, `SharedStrings.resx`
- `src/tools/illink/src/ILLink.CodeFix/`
- `src/tools/illink/test/ILLink.RoslynAnalyzer.Tests/`, `ILLink.RoslynAnalyzer.Tests.Generator/`
- `src/libraries/System.Runtime.InteropServices/gen/LibraryImportGenerator/Analyzers/`; `src/libraries/System.Text.RegularExpressions/gen/UpgradeToGeneratedRegexAnalyzer.cs`
- `src/libraries/Directory.Build.props`; `eng/illink.targets`; `eng/liveILLink.targets`; `docs/project/list-of-diagnostics.md`
