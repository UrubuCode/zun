# Trimming (ILLink) — análise de alcançabilidade (reachability)

## Mecanismo no .NET

### Visão geral

O **ILLink** (também chamado IL Linker / trimmer) remove de uma aplicação .NET o código e os metadados que não são alcançáveis a partir de um conjunto de *raízes*. Fica em `src/tools/illink/`:

| Caminho | Conteúdo |
|---|---|
| `src/tools/illink/src/linker/` | O trimmer (assembly `Mono.Linker`/`illink`): `Linker`, `Linker.Steps`, `Linker.Dataflow` |
| `src/tools/illink/src/ILLink.Shared/` | Código compartilhado entre o trimmer, o NativeAOT e o analisador Roslyn (DiagnosticId, TrimAnalysis, DataFlow) |
| `src/tools/illink/src/ILLink.RoslynAnalyzer/` | Analisador Roslyn em tempo de compilação (arquivo 05) |
| `src/tools/illink/src/ILLink.Tasks/` | Tarefa MSBuild (`LinkTask.cs`) |
| `src/tools/illink/test/` | `Mono.Linker.Tests`, `Mono.Linker.Tests.Cases`, `ILLink.RoslynAnalyzer.Tests` etc. |

O mesmo conceito, com outra implementação, existe no compilador NativeAOT: `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/Dataflow/` (o README ali diz que a lógica de dataflow "origina do IL Linker" e deve ser mantida em sincronia).

### Pipeline de passos

`src/tools/illink/src/linker/Linker/Pipeline.cs` define um `Pipeline` com lista de `IStep` e operações `PrependStep`, `AppendStep`, `AddStepBefore/After`, `ReplaceStep`, `RemoveStep`. Em `Driver.cs` (linhas ~1620-1627) a pipeline padrão acrescenta, entre outros: `ProcessReferencesStep`, `MarkStep`, `RemoveResourcesStep`, `ValidateVirtualMethodAnnotationsStep`, `ProcessWarningsStep`, `OutputWarningSuppressions`, `CodeRewriterStep`, `SweepStep`. Passos opcionais são inseridos conforme opções (por exemplo `ReflectionBlockedStep` após `MarkStep`, `SealerStep` antes de `OutputStep`; ver `Driver.cs` linhas ~906-916).

Em `src/tools/illink/src/linker/Linker.Steps/` há ainda `ResolveFromXmlStep` (raízes por descritor XML), `LinkAttributesStep` (atributos injetados por XML), `BodySubstituterStep` (substituições de corpo; ver arquivo 09), `RootAssemblyInputStep`, `CleanStep`, `OutputStep`.

### O coração: `MarkStep`

`MarkStep.cs` implementa o cálculo de alcançabilidade como **worklist até ponto fixo**. O método `Process(LinkContext)` chama `Initialize(); Process(); Complete();` (linha ~245). O laço principal, verificado nas linhas ~387-396:

```csharp
void Process()
{
    while (ProcessPrimaryQueue() ||
        ProcessMarkedPending() ||
        ProcessLazyAttributes() ||
        ProcessLateMarkedAttributes() ||
        MarkFullyPreservedAssemblies()) ;

    ProcessPendingTypeChecks();
}
```

E `ProcessPrimaryQueue` (linhas ~441-456) repete até a fila esvaziar:

```csharp
while (!QueueIsEmpty()) {
    ProcessQueue();
    ProcessMarkedTypesWithInterfaces();
    ProcessInterfaceMethods();
    ProcessDynamicCastableImplementationInterfaces();
    ProcessPendingBodies();
    DoAdditionalProcessing();
}
```

Ou seja: marcar um método enfileira o que seu corpo referencia; tipos marcados com interfaces e métodos de interface têm processamento adiado porque a alcançabilidade de métodos **virtuais e de interface depende de o tipo concreto ser instanciado/marcado**. O laço externo itera enquanto qualquer sub-processo produzir novo trabalho. Métodos relevantes: `MarkEntireType` (linha ~337), `ProcessMethod` (~3412), `ProcessInterfaceMethod` (~708), `ProcessDefaultImplementation` (~833), `ProcessInteropMethod` (~3723), `ProcessUnsafeAccessorMethod` (~3784).

### Raízes, ações por assembly e modos

`docs/design/tools/illink/trimmed-assemblies.md` descreve as opções expostas pelo SDK:

- `PostProcessAssemblies` — se o linker vê o assembly.
- `IsTrimmable` — se o assembly sofre *tree shaking* (caso contrário é raiz com ação `copy`).
- `TrimMode`: `copyused` (granularidade de assembly, padrão do .NET Core) ou `link` (granularidade de membro, agressivo). Pode ser metadado por assembly.
- `AssemblyMetadata("IsTrimmable","True")` (a partir do .NET 6) permite que o próprio assembly declare que é trimável.

`MarkFullyPreservedAssemblies` implementa a ação `Copy/Save`: assemblies com essas ações são marcados integralmente (e, se a ação padrão é `copy`, também os referenciados).

### Reflexão: alcançabilidade além do grafo estático

O grafo de chamadas estático não captura `Type.GetType("X").GetMethod("Y").Invoke(...)`. O documento `docs/design/tools/illink/reflection-flow.md` resume a abordagem:

- Reflexão **amigável a trimming** (maioria do código), **potencialmente não amigável** (depende de o valor do parâmetro ser conhecido) e **sempre não amigável**.
- Para o caso intermediário, faz análise de fluxo *intraprocedural* (valores `typeof(X)`, strings constantes) e, entre métodos, depende de **anotações** (`DynamicallyAccessedMembers`, arquivo 10).
- APIs como `Type.GetMethod`, `GetProperty`, `GetField`, `GetMember`, `GetNestedType` são reconhecidas como *intrínsecas*: se tipo e nome são conhecidos, só o membro exato é preservado; se o nome é desconhecido, todos os membros que casam são preservados.
- Registra explicitamente a história: o .NET Native tentou fazer "reflexão arbitrária funcionar" com análise cross-method e RD.XML, e mesmo assim ~20% do catálogo da Store tinha padrões arbitrários demais; por isso a decisão de *não* fazer tree-shaking em assemblies de usuário por padrão naquele produto.
- Non-goals explícitos: serializadores baseados em reflexão e injeção de dependência — "melhor resolvidos por source generators" (arquivo 12).

O motor dessa análise fica em `src/tools/illink/src/linker/Linker.Dataflow/`: `MethodBodyScanner.cs` (simulação de pilha IL), `ReflectionMethodBodyScanner.cs`, `HandleCallAction.cs`, `ReflectionMarker.cs`, `FlowAnnotations.cs`, `ValueNode.cs`, e os padrões `TrimAnalysis*Pattern.cs`. Os valores abstratos vivem em `ILLink.Shared/TrimAnalysis/` (`SystemTypeValue`, `KnownStringValue`, `NullValue`, `UnknownValue`, `MethodParameterValue`, `MethodReturnValue`, `GenericParameterValue`, …).

### Código gerado pelo compilador

`docs/design/tools/illink/compiler-generated-code-handling.md` (≈650 linhas) trata um problema real: atributos escritos pelo usuário (`RequiresUnreferencedCode`, `UnconditionalSuppressMessage`) **não** se propagam para corpos gerados (state machines de `async`, iteradores, lambdas, funções locais). O documento mostra o exemplo de `IL2026` emitido em `<TestBeforeAwait>d__1.MoveNext()` mesmo havendo supressão no método original. O trimmer precisa recuperar a relação "método de usuário → código gerado" (`CompilerGeneratedState.cs`, `CompilerGeneratedCallGraph.cs`, `HoistedLocalKey.cs`).

### Sweep e saída

Depois de `MarkStep`, `SweepStep` (`Linker.Steps/SweepStep.cs`) remove o que não foi marcado; `CodeRewriterStep` reescreve corpos (ex.: substituições); `OutputStep` grava. Avisos (IL2xxx) são coletados e filtrados em `ProcessWarningsStep`/`CheckSuppressionsStep`. Os códigos estão catalogados em `docs/tools/illink/error-codes.md` (~2000 linhas) e `ILLink.Shared/DiagnosticId.cs`.

## Por quê

- **Tamanho de aplicação**: o objetivo declarado em `docs/workflow/trimming/feature-switches.md` é "produzir saída menor removendo código inalcançável sob a condição do feature".
- **Alcançabilidade é a única noção segura**: remover apenas o que *nenhum caminho possível* alcança. Por isso o algoritmo é conservador (superaproxima o conjunto alcançável).
- **Separar análise de transformação**: `MarkStep` (decide) vs `SweepStep` (remove) vs `CodeRewriterStep` (reescreve).
- **Honestidade sobre reflexão**: em vez de prometer que tudo funciona, define um *subconjunto* compatível e **avisa** o restante (IL2xxx), garantindo que um programa sem avisos seja trimado com segurança.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Análise conservadora (superaproximação) | Correção: nunca remove o que é alcançável | Binário maior que o ótimo |
| Dataflow intraprocedural + anotações entre métodos | Escala (sem análise global) | Exige trabalho manual de anotação em bibliotecas |
| Reflexão fora do subconjunto = aviso, não erro | Não quebra builds de apps antigos | Aplicações com avisos podem quebrar em runtime |
| Marcação lazy com adiamento para virtuais/interfaces | Precisão (só mantém overrides de tipos instanciados) | Laço de ponto fixo complexo (`ProcessMarkedTypesWithInterfaces` etc.) |
| Granularidade `copyused` vs `link` | Opção para menor risco ou maior economia | Dois modos para testar e documentar |
| Dois motores (ILLink e NativeAOT) com lógica compartilhada via `ILLink.Shared` | Consistência de avisos | Exigência de manter em sincronia (README de `Compiler/Dataflow`) |

## Como aplicar na sua linguagem

1. **Projete a linguagem pensando em alcançabilidade estática desde o início.** Quanto mais o grafo de chamadas for visível sem reflexão, melhor o tree-shaking. Decida quais APIs são "dinâmicas" e dê-lhes marca explícita (ver arquivo 10).
2. **Implemente um `Mark` por worklist até ponto fixo** com fases separadas para: métodos normais, tipos instanciados, métodos virtuais/de interface (dependem de quais tipos concretos são alcançáveis) e atributos.
3. **Dê raízes explícitas**: ponto de entrada, descritores declarativos (XML/JSON/atributos), exports e raízes por biblioteca.
4. **Ofereça granularidade configurável** (por pacote/módulo): "preservar tudo" vs "por membro".
5. **Faça a análise de reflexão por fatos**: valores abstratos (`tipo conhecido`, `string constante`, `desconhecido`), intrínsecas para APIs de reflexão, e anotações para fronteiras entre funções.
6. **Emita avisos com ID estável e docs por código** (modelo `IL2026`/`error-codes.md`) e permita supressão localizada.
7. **Trate código gerado pelo compilador** como parte da função de origem desde o design: atributos/supressões precisam "viajar" para lambdas, state machines e funções locais. Se o seu compilador gera código sintético, publique a relação origem→gerado em metadados.
8. **Reuse a mesma análise** no IDE (analisador), no trimmer e no AOT: no .NET isso é feito com `ILLink.Shared`; planeje o código da análise como biblioteca independente do front-end de IL/AST.
9. **Teste por casos mínimos com expectativas anotadas**: o projeto `Mono.Linker.Tests.Cases` usa programas pequenos com atributos de expectativa (`Mono.Linker.Tests.Cases.Expectations`).

## Apêndice: arquivos de configuração do trimmer e mini-exemplo de marcação

### Os quatro arquivos `ILLink.*.xml` (de `docs/workflow/trimming/ILLink-files.md`)

| Arquivo | Função | Observação documentada |
|---|---|---|
| `ILLink.Descriptors.xml` | Forçar preservação de itens independentemente de referências | "Limitamos o uso ao mínimo"; típico para código não-IL (C/C++, JS) que chama IL; usar comentário justificando. `ILLink.Descriptors.LibraryBuild.xml` preserva só durante o build do repo |
| `ILLink.Substitutions.xml` | Substituir corpo de método por `throw` ou retorno constante | Implementa *feature switches* e constantes por plataforma (arquivo 09) |
| `ILLink.Suppressions.xml` | Suprimir avisos conhecidos durante o build do repo | Avisos suprimidos aqui **ainda** aparecem no app final; `[UnconditionalSuppressMessage]` nunca aparece; `...LibraryBuild.xml` suprime só no build do repo |
| `ILLink.LinkAttributes.xml` | Fazer o trimmer agir como se o item tivesse um atributo | Usado para remover atributos de design-time e, internamente, para injetar supressões |

O formato completo é referenciado como `docs/tools/illink/data-formats.md` nesses documentos. Em CoreLib existem `ILLink.Descriptors.*.xml`, `ILLink.LinkAttributes.Shared.xml`, `ILLink.Substitutions.*.xml` e `ILLink.Suppressions.LibraryBuild.xml` em `src/libraries/System.Private.CoreLib/src/ILLink/`.

### Mini-exemplo de marcação (ilustrativo)

```csharp
class Program {
    static void Main() {                 // raiz (ponto de entrada)
        var s = new Servico();
        s.Executar();                    // chamada estática: marca Servico e Executar
    }
}
class Servico : IExecutavel {
    public void Executar() { Auxiliar(); }
    void Auxiliar() { }
    public void NuncaChamado() { }       // não referenciado por ninguém
}
class Orfa { }                           // não referenciada
```

Resultado esperado em modo `link` (granularidade de membro): `Program`, `Main`, `Servico`, `Executar`, `Auxiliar`, `IExecutavel` e o ctor padrão ficam; `NuncaChamado` e `Orfa` são removidos. Em `copyused`, `Servico` e `Orfa` — pertencentes ao mesmo assembly usado — **ficam inteiros** (granularidade de assembly). Se `Executar` fosse chamado apenas via `typeof(Servico).GetMethod("NuncaChamado")` com string constante, a análise de reflexão (intrínseca de `GetMethod`) marcaria `NuncaChamado`; se o nome viesse de uma variável desconhecida, **todos os métodos que casam** seriam preservados (comportamento descrito em `reflection-flow.md`).

### Vocabulário da implementação

- `DependencyInfo` / `DependencyKind`: a razão pela qual algo foi marcado (usado para gerar relatórios de dependência).
- `MessageOrigin`: de onde veio um aviso (método, campo, assembly); é a base para supressão por origem.
- `Annotations` (estado compartilhado): ação por assembly (`Copy`, `Save`, `Link`, …), itens marcados, itens "preservados".
- `LinkContext`: configuração global (TrimAction, DefaultAction, assemblies carregados, pipeline).
- `IMarkHandler`/`MarkContext`: pontos de extensão para marcação customizada (`Initialize` chama `Context.Pipeline.InitializeMarkHandlers`).

## Referências

- `src/tools/illink/README.md`; `src/tools/illink/src/linker/Linker.Steps/MarkStep.cs`, `SweepStep.cs`, `CodeRewriterStep.cs`, `ResolveFromXmlStep.cs`
- `src/tools/illink/src/linker/Linker/Pipeline.cs`, `Driver.cs`
- `src/tools/illink/src/linker/Linker.Dataflow/` (scanner de dataflow)
- `src/tools/illink/src/ILLink.Shared/` (`DiagnosticId.cs`, `TrimAnalysis/`)
- `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/Dataflow/README.md`
- `docs/design/tools/illink/reflection-flow.md`, `trimmed-assemblies.md`, `compiler-generated-code-handling.md`
- `docs/tools/illink/error-codes.md`; `docs/workflow/trimming/feature-switches.md`, `ILLink-files.md`
- `src/tools/illink/test/Mono.Linker.Tests.Cases/`
