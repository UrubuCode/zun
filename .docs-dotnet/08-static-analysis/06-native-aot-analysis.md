# Análise para Native AOT — warnings de compatibilidade

## Mecanismo no .NET

### O problema

Em **Native AOT**, todo o código gerenciado é compilado antecipadamente para código nativo, sem JIT e sem carregar IL em runtime. Isso significa que:

1. **Código não referenciado estaticamente é removido** (mesma questão do trimming — arquivo 04);
2. **Código que exige geração de código em runtime** (`System.Reflection.Emit`, `Expression.Compile` com compilação, `MakeGenericType` sobre instanciações não vistas, etc.) **não pode funcionar**.

O segundo ponto é novo em relação ao trimming e tem atributo e família de avisos próprios. A análise, portanto, é: *"este programa depende de algo que só o runtime com JIT/IL dinâmico daria?"*.

### Atributo e códigos

- `RequiresDynamicCodeAttribute` em `src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/RequiresDynamicCodeAttribute.cs`. Alvos: `Method | Constructor | Class`, `Inherited = false`. Propriedades: `Message`, `Url`, e `ExcludeStatics` (quando `true`, a anotação não se aplica a membros estáticos). Comentário no código: "Isso permite que ferramentas entendam quais métodos são inseguros para chamar ao compilar antecipadamente".
- IDs em `src/tools/illink/src/ILLink.Shared/DiagnosticId.cs`, faixa "Dynamic code diagnostic ids":

| ID | Nome no enum | Significado |
|---|---|---|
| IL3050 | `RequiresDynamicCode` | Uso de membro com `RequiresDynamicCode` |
| IL3051 | `RequiresDynamicCodeAttributeMismatch` | Override/implementação com requisito diferente da base |
| IL3052 | `COMInteropNotSupportedInFullAOT` | COM interop não suportado em AOT completo |
| IL3053 | `AssemblyProducedAOTWarnings` | Assembly produziu avisos AOT (versão "single warn") |
| IL3054 | `GenericRecursionCycle` | Ciclo de recursão genérica |
| IL3055 | `CorrectnessOfAbstractDelegatesCannotBeGuaranteed` | Delegates abstratos |
| IL3056 | `RequiresDynamicCodeOnStaticConstructor` | Atributo em construtor estático não é permitido |
| IL3057 | `RequiresDynamicCodeOnEntryPoint` | Atributo no ponto de entrada |
| IL3058 | `ReferenceNotMarkedIsAotCompatible` | Referência a assembly não marcado como AOT-compatível |

Os mesmos arquivos também cobrem single-file (`IL3000`-`IL3005`, `RequiresAssemblyFiles`) e trimming (`IL2xxx`). Documentação de cada código: `docs/tools/illink/error-codes.md`.

### Dois produtores dos mesmos avisos

1. **Analisador Roslyn** (`ILLink.RoslynAnalyzer/RequiresDynamicCodeAnalyzer.cs`, mesmo esqueleto de `RequiresAnalyzerBase`; habilitado por `EnableAotAnalyzer`, ver `Microsoft.NET.ILLink.Analyzers.props`). Avisa no editor/build, sem compilar para AOT. No repo, `src/libraries/Directory.Build.props` (linhas ~66-69) define `IsAotCompatible=true` por padrão para projetos de código-fonte e referência, e `EnableAotAnalyzer=true` para projetos de teste.
2. **O compilador ILCompiler** (`src/coreclr/tools/aot/ILCompiler.Compiler/`). Sua pasta `Compiler/Dataflow/` contém o mesmo tipo de motor do ILLink: `ReflectionMethodBodyScanner.cs`, `HandleCallAction.cs`, `ReflectionMarker.cs`, `FlowAnnotations.cs`, `TrimAnalysis*Pattern.cs`, `GenericArgumentDataFlow.cs`, `AttributeDataFlow.cs`. O README da pasta diz que a lógica "origina do IL Linker" e deve ser mantida em sincronia. `UsageBasedMetadataManager.cs` (linhas ~48, ~659-672) mapeia os atributos `Requires*` para os IDs de mismatch e emite `RequiresDynamicCodeOnStaticConstructor` etc. via `Logger`.

Como o analisador Roslyn "não tem insight sobre a estratégia de compilação do código gerado pelo compilador" (palavras de `docs/design/tools/illink/feature-attribute-semantics.md`), a análise de IL pode produzir **avisos adicionais**, por exemplo para acesso por reflexão a código gerado.

### Controle do usuário sobre o ruído

`src/coreclr/tools/aot/ILCompiler/ILCompilerRootCommand.cs` (linhas ~135-150) expõe as opções de linha de comando:

| Opção | Efeito (descrição no código) |
|---|---|
| `--nowarn` | Desabilita avisos específicos |
| `--singlewarn` | Um único aviso AOT/trimming por assembly |
| `--singlewarnassembly` / `--nosinglewarnassembly` | Liga/desliga o modo "single warn" para assemblies específicos |
| `--notrimwarn` | Desabilita avisos relacionados a trimming |
| `--noaotwarn` | Desabilita avisos relacionados a AOT |
| `--warnaserror` / `--warnaserr` | Trata todos / alguns avisos como erro |

O modo "single warn" existe porque uma biblioteca de terceiros não anotada pode produzir centenas de avisos; o IL3053 `AssemblyProducedAOTWarnings` sintetiza isso.

### Feature switches, `RuntimeFeature` e guardas

A API `RuntimeFeature.IsDynamicCodeSupported` e `IsDynamicCodeCompiled` permite ao código **escolher caminho**. Na implementação NativeAOT (`src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeFeature.NativeAot.cs`):

```csharp
[FeatureSwitchDefinition("System.Runtime.CompilerServices.RuntimeFeature.IsDynamicCodeSupported")]
public static bool IsDynamicCodeSupported => false;

[FeatureGuard(typeof(RequiresDynamicCodeAttribute))]
public static bool IsDynamicCodeCompiled => false;
```

O compilador usa estes atributos (`FeatureSwitchDefinitionAttribute`, `FeatureGuardAttribute`, ambos em `System/Diagnostics/CodeAnalysis/`) em `ILCompiler.Compiler/Compiler/SubstitutionProvider.cs`: se a propriedade tem `FeatureSwitchDefinition`, o valor vem do conjunto de *switches* do compilador; se tem `FeatureGuard(typeof(RequiresDynamicCodeAttribute))` (ou `RequiresUnreferencedCode`/`RequiresAssemblyFiles`), a propriedade é tratada como guarda `true` que protege chamadas a APIs anotadas. Quando o switch é uma constante conhecida, **os ramos guardados são removidos** e deixam de gerar avisos (`docs/design/tools/illink/feature-checks.md`: "ILLink e ILCompiler removem inteiramente os ramos guardados por um feature check para um feature desabilitado"). Exemplo canônico:

```csharp
if (RuntimeFeature.IsDynamicCodeSupported)
    UsarReflectionEmit();          // sem aviso, e removido no AOT
else
    UsarCaminhoEstatico();
```

O MSBuild `DynamicCodeSupport` (em `docs/workflow/trimming/feature-switches.md`) altera `IsDynamicCodeSupported` para false, "para permitir testar código de fallback AOT-safe sem publicar com Native AOT".

### Genéricos e instanciação

Nem todo problema AOT é "usa reflexão". Para tipos genéricos, o AOT precisa conhecer cada instanciação sobre tipos-valor. `MakeGenericType`/`MakeGenericMethod` são tratados como **intrínsecas** (`IntrinsicId.Type_MakeGenericType`, `IntrinsicId.MethodInfo_MakeGenericMethod` aparecem no `RequiresDynamicCodeAnalyzer.cs`, linhas ~57 e ~79 e em `ILLink.Shared/TrimAnalysis/IntrinsicId.cs`). Quando o tipo-argumento ou o genérico não são conhecidos, o compilador avisa que a instanciação pode requerer código dinâmico. `GenericRecursionCycle` (IL3054) sinaliza o caso patológico.

## Por quê

- **Sem JIT no destino**: a ausência de geração de código em runtime é um *fato de plataforma* (iOS, WASM, consoles, binários pequenos). Código que depende disso falha em runtime com `PlatformNotSupportedException` ou comportamento incorreto; o aviso transforma isso em erro de build.
- **Reuso do trimmer**: a mesma análise que decide alcançabilidade também decide "isto precisa de código dinâmico?" — um único motor de dataflow com dois grupos de atributos.
- **Contrato de biblioteca**: bibliotecas marcam `IsAotCompatible`; consumidores só precisam confiar em ausência de avisos (IL3058 aponta referências não marcadas).
- **Feature guards**: permitem uma única base de código com fallback, sem `#if`.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Aviso, não erro, por padrão | Compatibilidade de migração | Binário publicado pode falhar em runtime se avisos forem ignorados |
| Dois produtores (Roslyn + ILCompiler) | Feedback rápido + análise completa de IL | Divergência possível; custo de manter sincronia (`Compiler/Dataflow/README.md`) |
| Atributo por método/classe, propaga a "fatias" do código | Granularidade e supressão local | Regras complicadas para lambdas, state machines, estáticos (`feature-attribute-semantics.md`) |
| Guardas via propriedades booleanas conhecidas | Remove código morto e silencia avisos | Só funciona para padrões reconhecidos (`!`, `== true`, `&&`, etc. em `feature-checks.md`) |
| Modo `--singlewarn` | Menos ruído | Esconde problemas individuais |
| Suporte parcial de dinâmicos via análise de `MakeGenericType` | Mais programas funcionam | Análise precisa de conhecer tipos; casos desconhecidos viram aviso |

## Como aplicar na sua linguagem

1. **Decida se haverá geração de código em runtime**. Se não (ou se for opcional), crie desde o início uma noção de "capacidade" (`IsDynamicCodeSupported`) e uma anotação (`RequiresDynamicCode`) separadas de `RequiresUnreferencedCode`; no .NET são capacidades ortogonais.
2. **Faça o compilador AOT incorporar o mesmo motor de dataflow** do seu analisador de fonte. Se a sua stdlib usa reflexão, trate `MakeGenericType`-like como intrínsecas.
3. **Exponha propriedades booleanas "guarda"** que o compilador conheça como constantes e que *removam* ramos: `if (Platform.SupportsX) ... else ...`. Documente os padrões de guarda aceitos (ver `feature-checks.md`).
4. **Ofereça controle de ruído**: `--nowarn`, `--warnaserror`, "um aviso por assembly".
5. **Defina IDs estáveis** para as categorias: atributo desalinhado entre base/derivada, atributo no construtor estático/ponto de entrada, referência não marcada como compatível.
6. **Regras para atributo em override**: o .NET exige que um override tenha o mesmo requisito da base (IL3051); copie essa regra.
7. **Considere um modo "simulado"** (no .NET, `DynamicCodeSupport=false`) para testar o fallback sem toolchain AOT.
8. **Marque a stdlib**: todo componente da biblioteca padrão deve ser compatível ou anotado; ao distribuir, informe `IsAotCompatible`.

## Apêndice: exemplos de uso e cenário de ponta a ponta

### Tabela comparativa: trimming vs AOT vs single-file

| Aspecto | Trimming (ILLink) | Native AOT | Single-file |
|---|---|---|---|
| Atributo de requisito | `RequiresUnreferencedCode` | `RequiresDynamicCode` | `RequiresAssemblyFiles` |
| ID principal | IL2026 | IL3050 | IL3002 |
| Analisador habilitado por | `EnableTrimAnalyzer` | `EnableAotAnalyzer` | `EnableSingleFileAnalyzer` |
| Guarda em runtime | feature switches (`IsUnreferencedCodeSupported` é tratado como guarda "para teste" no analisador) | `RuntimeFeature.IsDynamicCodeSupported` / `IsDynamicCodeCompiled` | — |
| Pergunta | "algum código necessário foi removido?" | "alguma geração de código é necessária em runtime?" | "alguma API assume arquivos de assembly em disco?" |

(O comentário sobre `IsUnreferencedCodeSupported` está em `RequiresUnreferencedCodeAnalyzer.IsRequiresCheck`; as propriedades MSBuild estão em `Microsoft.NET.ILLink.Analyzers.props`.)

### Cenário ponta a ponta

```csharp
// Biblioteca marcada IsAotCompatible
public static class Mapper
{
    [RequiresDynamicCode("Usa Expression.Compile")]
    public static Func<T,U> Compilar<T,U>() { /* ... */ }

    public static Func<T,U> Obter<T,U>()
    {
        if (RuntimeFeature.IsDynamicCodeSupported)
            return Compilar<T,U>();      // sem IL3050: está guardado; no AOT o ramo é removido
        return Interpretado<T,U>();       // caminho estático
    }
}

// Aplicação
Mapper.Compilar<A,B>();                   // IL3050 (analisador no editor; ILCompiler no publish)
```

Efeitos: (1) no editor, o analisador mostra IL3050 no ponto de chamada de `Compilar`; (2) em `publish` AOT, o ILCompiler analisa o IL com o mesmo conjunto de regras e reporta o aviso; (3) com `IsDynamicCodeSupported` constante `false` no AOT (confirmado em `RuntimeFeature.NativeAot.cs`), o ramo guardado em `Obter` desaparece e nada é compilado para `Compilar`.

### Regras de herança do atributo (do documento de semântica)

- Override com requisito diferente da base: `IL3051` (`RequiresDynamicCodeAttributeMismatch`).
- Atributo em construtor estático: `IL3056` (`RequiresFeature` em construtor estático é "não suportado" segundo `feature-attribute-semantics.md`).
- Atributo no ponto de entrada: `IL3057`.
- Tipo com requisito: afeta membros estáticos e construtores (e todos os membros em struct).

### Onde olhar quando um aviso AOT aparece

1. `docs/tools/illink/error-codes.md` — texto e exemplo do código.
2. `ILCompiler.Compiler/Compiler/Dataflow/HandleCallAction.cs` — como a chamada à API de reflexão é interpretada.
3. `UsageBasedMetadataManager.cs` — onde `Logger.LogWarning` é chamado para atributos nos construtores estáticos.
4. Opções do `ILCompilerRootCommand.cs`: `--singlewarn`, `--nowarn` para triagem.
5. `src/coreclr/nativeaot/docs/troubleshooting.md` — guia de solução de problemas (o arquivo existe; não o detalhei aqui).

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/RequiresDynamicCodeAttribute.cs`, `FeatureGuardAttribute.cs`, `FeatureSwitchDefinitionAttribute.cs`
- `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeFeature.NativeAot.cs`
- `src/tools/illink/src/ILLink.Shared/DiagnosticId.cs`
- `src/tools/illink/src/ILLink.RoslynAnalyzer/RequiresDynamicCodeAnalyzer.cs`
- `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/Dataflow/` (+ `README.md`), `UsageBasedMetadataManager.cs`, `SubstitutionProvider.cs`
- `src/coreclr/tools/aot/ILCompiler/ILCompilerRootCommand.cs`
- `docs/design/tools/illink/feature-attribute-semantics.md`, `feature-checks.md`; `docs/workflow/trimming/feature-switches.md`; `docs/tools/illink/error-codes.md`
- `src/coreclr/nativeaot/docs/` (README, compiling.md, troubleshooting.md)
