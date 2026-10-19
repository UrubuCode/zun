# Análise dirigida por atributos (RequiresUnreferencedCode, DynamicallyAccessedMembers)

## Mecanismo no .NET

### Ideia central

A análise de fluxo é **intraprocedural**; para atravessar fronteiras de método, campo ou tipo sem análise global, o .NET pede ao programador que **declare contratos com atributos**. O trimmer, o compilador NativeAOT e o analisador Roslyn leem esses atributos e verificam os contratos por interpretação abstrata (arquivos 04, 05, 06). Este documento descreve a "linguagem de anotações" em si.

Todos os atributos abaixo ficam em `src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/` e usam o padrão `#if SYSTEM_PRIVATE_CORELIB public #else internal #endif` — públicos em CoreLib e **internos** (cópia privada) em outros projetos do repo que precisam do mesmo atributo.

### 1. `RequiresUnreferencedCodeAttribute` (RUC)

Declara: "este membro é incompatível com trimming (usa reflexão que a análise não consegue seguir)". Definição (arquivo `RequiresUnreferencedCodeAttribute.cs`):

- Alvos: `Method | Constructor | Class`, `Inherited = false`.
- `RequiresUnreferencedCodeAttribute(string message)`, `Message`, `Url`, `ExcludeStatics` ("quando `true` indica que a anotação não se aplica a membros estáticos").
- Exemplo real: `Type.cs` linha ~769: `[RequiresUnreferencedCode("If some of the generic arguments are annotated (either with DynamicallyAccessedMembersAttribute, or generic constraints), trimming can't validate that the requirements of those annotations are met.")]`.

Efeito: **todo chamador** de um membro RUC, fora de um escopo onde a feature já é exigida, recebe `IL2026` (`DiagnosticId.RequiresUnreferencedCode = 2026`). Exemplo de `docs/tools/illink/error-codes.md`:

```csharp
[RequiresUnreferencedCode("Use 'MethodFriendlyToTrimming' instead", Url="http://help/unreferencedcode")]
void MethodWithUnreferencedCodeUsage() { }
void TestMethod() { MethodWithUnreferencedCodeUsage(); /* IL2026 */ }
```

Atributos irmãos com **mesma semântica**: `RequiresDynamicCodeAttribute` (IL3050, arquivo 06) e `RequiresAssemblyFilesAttribute` (IL3002, single-file), mais `RequiresUnsafeAttribute` (migração de segurança de memória). A semântica comum é especificada como "feature attributes" em `docs/design/tools/illink/feature-attribute-semantics.md`.

### 2. `DynamicallyAccessedMembersAttribute` (DAM) e `DynamicallyAccessedMemberTypes`

DAM declara: "o valor `Type` (ou string com nome de tipo) neste local **terá seus membros acessados dinamicamente** — preserve-os / exija que a origem do valor garanta isso". Definição (`DynamicallyAccessedMembersAttribute.cs`):

- Alvos: `Field | ReturnValue | GenericParameter | Parameter | Property | Method | Class | Interface | Struct`, `Inherited = false`; tem `[CompilerLoweringPreserve]`.
- Construtor `DynamicallyAccessedMembersAttribute(DynamicallyAccessedMemberTypes memberTypes)`.

O enum `DynamicallyAccessedMemberTypes` é um conjunto de *flags* (`DynamicallyAccessedMemberTypes.cs`), confirmado: `None = 0`, `PublicParameterlessConstructor = 0x0001`, `PublicConstructors = 0x0002 | PublicParameterlessConstructor`, `NonPublicConstructors = 0x0004`, `PublicMethods = 0x0008`, `NonPublicMethods = 0x0010`, `PublicFields = 0x0020`, `NonPublicFields = 0x0040`, `PublicNestedTypes = 0x0080`, `NonPublicNestedTypes = 0x0100`, `PublicProperties = 0x0200`, `NonPublicProperties = 0x0400`, `PublicEvents = 0x0800`, `NonPublicEvents = 0x1000`, `Interfaces = 0x2000`, variantes `*WithInherited` (bits 0x4000 a 0x200000), `AllConstructors`, e outras combinações (`All`). Note a composição: `PublicConstructors` **inclui** `PublicParameterlessConstructor` (subconjunto como bit).

Uso real: `System.Activator` (`Activator.cs` linhas 25-40):

```csharp
public static object? CreateInstance([DynamicallyAccessedMembers(DynamicallyAccessedMemberTypes.PublicParameterlessConstructor)] Type type)
public static object? CreateInstance([DynamicallyAccessedMembers(DynamicallyAccessedMemberTypes.PublicConstructors)] Type type, params object?[]? args)
```

O contrato: quem chama `CreateInstance(type)` deve garantir que `type` tenha, no mínimo, o construtor público sem parâmetros preservado. O trimmer assim preserva o ctor exatamente para os tipos que *flowam* até esse argumento.

### 3. A regra de subtipagem das anotações (verificação)

Do `error-codes.md`, `IL2067`: "Os requisitos declarados no local alvo devem ser atendidos pelos declarados no valor de origem. O valor de origem **pode declarar mais requisitos** que o alvo, se necessário". Ou seja, a ordem é por inclusão de flags: `source ⊇ target`. Exemplo:

```csharp
void NeedsPublicConstructors([DynamicallyAccessedMembers(PublicConstructors)] Type type) {}
void TestMethod(Type type) {
    NeedsPublicConstructors(type);   // IL2067: 'type' não tem anotação compatível
}
```

A família de códigos varia conforme a **origem** e o **destino**: parâmetro→parâmetro (`IL2067`), parâmetro→retorno (`IL2068`), retorno→parâmetro (`IL2072`), campo, `this` (`IL2070`, `IL2075`), parâmetro genérico (`IL2087`, `IL2091`)… O motor está em `ILLink.Shared/TrimAnalysis/RequireDynamicallyAccessedMembersAction.cs` e `HandleCallAction.cs`; o `FlowAnnotations.cs` (trimmer, NativeAOT e analisador) carrega as anotações de cada símbolo.

Os valores do lattice que carregam anotações (em `ILLink.Shared/TrimAnalysis/`): `MethodParameterValue`, `MethodReturnValue`, `FieldValue`, `GenericParameterValue`, `ValueWithDynamicallyAccessedMembers` (base), `SystemTypeValue` (um `typeof(X)` concreto), `KnownStringValue` (string constante usada em `GetType("...")`), `ArrayOfAnnotatedSystemTypeValue`, `NullableSystemTypeValue`, `RuntimeTypeHandleValue`, `UnknownValue`.

### 4. DAM em tipo, interação com RUC

`docs/design/tools/illink/DAM-on-type-and-RUC-interaction.md`:

- DAM num tipo "cria efetivamente um acesso por reflexão a todos os membros afetados"; deve gerar avisos como se cada membro fosse acessado via `GetMethod` etc.
- **RUC no método** suprime avisos do corpo e dos atributos desse método, mas **não** avisos sobre o *uso* do método.
- **RUC no tipo** equivale externamente a RUC em todos os membros estáticos e construtores; suprime avisos como se todos os métodos tivessem RUC; **não** suprime avisos de tipos aninhados.
- **RUC no tipo não suprime os avisos causados por DAM no tipo** (decisão explícita, com justificativa: o DAM do tipo "é efetivamente público" e faz parte do contrato do tipo; suprimir geraria quebras inesperadas). Códigos usados nas amostras: `IL2112`, `IL2113`, `IL2114`.
- Origem do aviso: o membro em si (se tipo ou base anotados acessam o membro) ou o tipo que adicionou o DAM (se acessa membro da base) — "avisar sobre a coisa que é culpada".

### 5. Outros atributos do ecossistema

- `UnconditionalSuppressMessageAttribute(string category, string checkId)` — `AttributeTargets.All`, `AllowMultiple = true`. Suprime um aviso do trimmer/AOT que o autor sabe ser falso positivo (exige `Justification`). A política do repo, em `docs/workflow/trimming/ILLink-files.md`: novos avisos devem ser resolvidos com `[DynamicallyAccessedMembers]` ou `[RequiresUnreferencedCode]`; falso positivo → `[UnconditionalSuppressMessage]`; **nenhuma nova entrada** em `ILLink.Suppressions.xml`.
- `DynamicDependencyAttribute` (construtores com `memberSignature`, `Type`/`typeName+assemblyName` ou `DynamicallyAccessedMemberTypes`) — declara que um membro depende **por reflexão** de outro, preservando-o.
- `FeatureSwitchDefinitionAttribute` e `FeatureGuardAttribute(Type featureType)` — ligam uma propriedade booleana a uma *feature* para dobrar constantes e silenciar avisos (arquivos 06 e 09).
- `ExperimentalAttribute`, `StringSyntaxAttribute`, `ConstantExpectedAttribute`, `SetsRequiredMembersAttribute`, `UnscopedRefAttribute` — mesma pasta; atributos que informam análises (experimental, regex/JSON em strings, constantes obrigatórias…).

### 6. Escopo da anotação: "feature available" e "feature requirement"

`feature-attribute-semantics.md` formaliza para qualquer `RequiresFeature`:

- Métodos/ctors com a anotação estão em escopo *feature available* (chamar outros métodos com requisito não gera aviso).
- Em **classe** com a anotação: métodos estáticos, ctors, campos/propriedades/eventos estáticos ganham requisito (membros de instância ficam disponíveis); em **struct**: todos os membros. Não se estende a tipos aninhados nem a membros herdados.
- State machines (async/iterator) e lambdas/funções locais herdam o escopo do método contendo; as ferramentas de IL divergem um pouco (lambdas herdam do método de usuário).
- Validação: `RequiresFeature` em static ctor é proibido; **override não pode divergir** do requisito da base (nem para mais nem para menos); acesso fora do escopo é "não permitido" (gera aviso).
- `feature-checks.md`: padrões de guarda reconhecidos (`if (Feature.IsSupported) Feature.Run();`, negação, `== true/false`, combinações).

## Por quê

- **Escalar análise sem análise global**: um contrato por fronteira permite verificar método a método e biblioteca a biblioteca (o "trimming de bibliotecas" depende de anotar APIs públicas).
- **Transformar falhas silenciosas em avisos de build**: reflexão que quebra em app trimado vira `IL2026/IL2067…`.
- **Contratos visíveis ao consumidor**: o atributo está nos metadados; o consumidor de uma biblioteca sem acesso ao código-fonte ainda recebe o aviso.
- **Mínimo de preservação**: DAM com flags finas (`PublicParameterlessConstructor`) permite manter só o necessário.
- **Mesmo vocabulário nas três ferramentas**: reuso de `ILLink.Shared`.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Atributos em vez de análise interprocedural | Escalável e modular | Carga de anotação no autor da biblioteca; anotação errada = bugs só em runtime |
| RUC como "porta de escape" | Qualquer API incompatível pode ser marcada | Contamina chamadores (propagação em cascata de RUC) |
| DAM por flags | Precisão fina | Enum grande e sutil (`WithInherited`), difícil de acertar |
| Regra `source ⊇ target` | Simples de checar | Obriga a propagar anotações por toda a cadeia (parâmetros, campos, genéricos) |
| Anotações em tipo (DAM no tipo) | Contrato do tipo explícito | Interação complexa com RUC; documento de 168 linhas só para as regras |
| Supressão por atributo | Resolve falsos positivos locais | Suprimir mal é perigoso (o doc de DAM/RUC alerta para isso) |
| Escopo "feature available" por tipo | Cobertura de async/lambdas | Regras detalhadas; ferramentas de IL divergem das de fonte |

## Como aplicar na sua linguagem

1. **Planeje desde o início uma linguagem de anotações de "capacidades"**: um atributo `Requires(<feature>)` genérico + uma lista fechada (ou extensível) de features (reflexão, código dinâmico, filesystem, rede…). A especificação `feature-attribute-semantics.md` é um bom ponto de partida: defina *feature requirement* vs *feature available* e como se propagam a tipos aninhados, lambdas, geradores.
2. **Anotações de fluxo de valores sensíveis** (equivalente a DAM): tipos/valores que carregam "o que será acessado" como flags composicionais com ordem parcial `⊇`. Defina o enum com subconjunto embutido (como `PublicConstructors ⊇ PublicParameterlessConstructor`).
3. **Pontos de verificação**: parâmetros, retornos, campos, `this`, parâmetros genéricos. Dê um código de diagnóstico distinto por combinação origem/destino (modelo `IL2067…IL2091`).
4. **Regras de herança/override**: sem divergência de requisito entre base e derivada; decida o que acontece com interfaces.
5. **Escape hatches explícitos**: `SuppressMessage` com justificativa obrigatória; `DynamicDependency` para dependências que o analisador não vê.
6. **Provedor de atributos externo**: o trimmer aceita injeção por XML (`ILLink.LinkAttributes.xml`, schema em `ILLink.Shared/ILLink.LinkAttributes.xsd`) para anotar código que você não controla. Prevê isso para bibliotecas de terceiros.
7. **Ofereça code fixes** (modelo `DAM-code-fix.md`: o analisador envia id, local, argumento e declaração-alvo nas `properties`; o fixer gera o atributo).
8. **Preveja a "interação" entre os atributos** logo: o repo teve que escrever documentos separados para DAM×RUC, código gerado e escopo aninhado — projete as regras de forma ortogonal para evitar isso.

## Apêndice: cenário completo de propagação de anotações

### Cadeia de chamadas com DAM

```csharp
// Camada baixa (CoreLib): consome o Type via reflexão
public static object? CreateInstance(
    [DynamicallyAccessedMembers(DynamicallyAccessedMemberTypes.PublicParameterlessConstructor)] Type type);

// Camada média (sua biblioteca): repassa o requisito
public static object Criar(
    [DynamicallyAccessedMembers(DynamicallyAccessedMemberTypes.PublicParameterlessConstructor)] Type t)
    => Activator.CreateInstance(t)!;           // OK: origem declara o mesmo requisito

// Camada alta (app): fornece tipos concretos
var o = Criar(typeof(Servico));                // OK: typeof(Servico) é SystemTypeValue; o trimmer preserva Servico..ctor()
var p = Criar(Type.GetType(nome));             // Aviso (IL2057 na família Unrecognized value passed to Type.GetType; conforme error-codes.md): nome desconhecido
```

Observações:

- Na camada média, se faltar o atributo em `t`, o aviso é da família `IL2067` ("parâmetro não tem anotações compatíveis").
- Para genéricos, `[DynamicallyAccessedMembers(...)] T` em parâmetro de tipo gera avisos da família `IL2087/IL2091` quando o argumento de tipo não satisfaz o requisito.
- O trimmer reconhece `typeof(Servico)` e `Servico.GetType()` (valor de um tipo estático) como valores conhecidos; um `Type` vindo de string não constante vira `UnknownValue`.

(`IL2057` confere com `docs/tools/illink/error-codes.md`: "Unrecognized value passed to the parameter 'typeName' of method 'System.Type.GetType'".)

### Escopo e propagação do RUC

```csharp
[RequiresUnreferencedCode("Usa reflexão arbitrária")]
public static void Raiz() { Interno(); }        // sem aviso: corpo de método RUC está em escopo "feature available"

public static void Chamador() {
    Raiz();                                      // IL2026: chamada fora do escopo
}

[RequiresUnreferencedCode("Propaga")]
public static void Outro() { Raiz(); }          // sem aviso: Outro também exige RUC
```

Duas saídas para o aviso em `Chamador`: (a) propagar o requisito com `[RequiresUnreferencedCode]` em `Chamador`; (b) suprimir com `[UnconditionalSuppressMessage("Trimming","IL2026", Justification="...")]` se o autor sabe que é seguro. Em (a) o aviso "sobe" na árvore de chamadas; em (b) o autor assume a responsabilidade.

### Tabela de decisão: qual atributo usar?

| Situação | Atributo |
|---|---|
| Método usa reflexão sobre tipo desconhecido e não dá para anotar | `RequiresUnreferencedCode` |
| Método recebe `Type` e acessa seus membros por reflexão | `DynamicallyAccessedMembers` no parâmetro |
| Método precisa de `Reflection.Emit`/`MakeGenericType` não conhecido | `RequiresDynamicCode` |
| Código depende de `Assembly.Location` | `RequiresAssemblyFiles` |
| Falso positivo conhecido | `UnconditionalSuppressMessage` com justificativa |
| Dependência por reflexão de membro conhecido | `DynamicDependency` |
| Propriedade que guarda código dependente de feature | `FeatureGuard`/`FeatureSwitchDefinition` |

### Pontos de verificação que o trimmer aplica (lista de "alvos" de uma atribuição anotada)

Parâmetro de método, valor de retorno, campo, `this` (ex.: chamar `GetMethod` num `Type` anotado), parâmetro de tipo genérico, propriedade (via acessores), e atributos personalizados (arquivo `AttributeDataFlow.cs`: o trimmer analisa argumentos de construtores de atributo que são `Type`/`string` anotados). A existência de `AttributeDataFlow.cs` e `GenericArgumentDataFlow.cs` em `Linker.Dataflow` e `ILCompiler.Compiler/Compiler/Dataflow` confirma esses dois alvos.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Diagnostics/CodeAnalysis/RequiresUnreferencedCodeAttribute.cs`, `RequiresDynamicCodeAttribute.cs`, `RequiresAssemblyFilesAttribute.cs`, `RequiresUnsafeAttribute.cs`, `DynamicallyAccessedMembersAttribute.cs`, `DynamicallyAccessedMemberTypes.cs`, `DynamicDependencyAttribute.cs`, `UnconditionalSuppressMessageAttribute.cs`, `FeatureGuardAttribute.cs`, `FeatureSwitchDefinitionAttribute.cs`
- `src/libraries/System.Private.CoreLib/src/System/Activator.cs`, `Type.cs` (exemplos de uso)
- `src/tools/illink/src/ILLink.Shared/TrimAnalysis/` (`RequireDynamicallyAccessedMembersAction.cs`, `HandleCallAction.cs`, `FlowAnnotations.cs`, valores)
- `src/tools/illink/src/ILLink.RoslynAnalyzer/TrimAnalysis/DAM-code-fix.md`
- `docs/design/tools/illink/DAM-on-type-and-RUC-interaction.md`, `feature-attribute-semantics.md`, `feature-checks.md`, `reflection-flow.md`
- `docs/tools/illink/error-codes.md` (IL2026, IL2067, IL2068, IL2070, IL2072, IL2075, IL2087)
- `docs/workflow/trimming/ILLink-files.md`
