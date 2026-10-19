# dynamic e o DLR (despacho dinâmico)

## Mecanismo no .NET

Em C#, uma expressão do tipo `dynamic` desliga a checagem estática: cada operação (chamada de método, acesso a membro, operador, conversão, indexação) é compilada para um **call site** que é resolvido em runtime. A infraestrutura é o **DLR** (Dynamic Language Runtime), implementado em `System.Linq.Expressions` (call sites e binders) mais `Microsoft.CSharp` (regras específicas do C#).

### Componentes (arquivos reais)

| Componente | Arquivo |
|---|---|
| `CallSite` / `CallSite<T>` | `src/libraries/System.Linq.Expressions/src/System/Runtime/CompilerServices/CallSite.cs` |
| `CallSiteBinder` (base dos binders) | `.../CompilerServices/CallSiteBinder.cs` |
| Cache L2 de regras | `.../CompilerServices/RuleCache.cs` |
| Operações auxiliares | `CallSiteOps.cs`, `CallSiteHelpers.cs` |
| Binders abstratos por operação | `src/libraries/System.Linq.Expressions/src/System/Dynamic/`: `GetMemberBinder`, `SetMemberBinder`, `InvokeMemberBinder`, `InvokeBinder`, `BinaryOperationBinder`, `UnaryOperationBinder`, `ConvertBinder`, `GetIndexBinder`, `SetIndexBinder`, `CreateInstanceBinder`, `DeleteMemberBinder`, `DeleteIndexBinder` |
| Meta-objetos | `DynamicMetaObject.cs`, `DynamicMetaObjectBinder.cs`, `IDynamicMetaObjectProvider.cs`, `BindingRestrictions.cs` |
| Objetos dinâmicos prontos | `DynamicObject.cs`, `ExpandoObject.cs`, `ExpandoClass.cs` |
| Binders C# | `src/libraries/Microsoft.CSharp/src/Microsoft/CSharp/RuntimeBinder/`: `Binder.cs` (fábrica `Microsoft.CSharp.RuntimeBinder.Binder`), `CSharpBinaryOperationBinder`, `CSharpInvokeMemberBinder`, `CSharpUnaryOperationBinder`, `CSharpInvokeBinder`, `RuntimeBinder.cs` |
| Nó de árvore | `DynamicExpression.cs`, `DynamicExpressionVisitor.cs` |

### Ciclo de uma chamada dinâmica

Um `CallSite<T>` (T é um tipo de delegate) mantém três níveis de cache, confirmados nos comentários de `CallSite.cs`:

- **Level 0**: o delegate `Target` do site, especializado pela história do site;
- **Level 1**: array de regras recentes do próprio site, tamanho máximo `MaxRules = 10` (`CallSite.cs` ~linha 238);
- **Level 2**: `RuleCache<T>` associado ao **binder**, compartilhado entre sites, `MaxRules = 128`, com posição de inserção `MaxRules / 2` (`RuleCache.cs`).

Se todos falham, `UpdateAndExecute` invoca o binder ("This binder is invoked by the UpdateAndExecute below if all Level 0, Level 1 and Level 2 caches experience cache miss", comentário em `CallSite.cs` ~linha 58). O binder (`Bind`) devolve uma **expressão** — uma árvore de `System.Linq.Expressions` — contendo: (a) as restrições (`BindingRestrictions`: "esta regra vale se o objeto for do tipo X") e (b) o corpo do despacho. O DLR compila essa regra (arquivo 07) para um delegate `T` e instala como novo `Target`; chamadas subsequentes com a mesma forma de operandos pagam só o teste de restrição.

### IDynamicMetaObjectProvider

Objetos podem assumir o próprio despacho implementando `IDynamicMetaObjectProvider` (ou herdando `DynamicObject`/`ExpandoObject`): o binder pergunta ao `DynamicMetaObject` do alvo antes de cair em reflexão (`FallbackXxx`). Isto permite linguagens (Python, JS) e objetos como `ExpandoObject` (propriedades em um dicionário versionado: `ExpandoClass`) a definirem sua semântica.

### C# runtime binder

O binder C# reproduz as regras do compilador (overload resolution, conversões, operadores definidos por usuário, lifting) em runtime. Ele analisa os **tipos em runtime dos argumentos** (`CSharpArgumentInfo.cs`, flags `CSharpBinderFlags`, `CSharpCallFlags`), inclusive tipos estáticos de contexto para acessibilidade.


### Exemplo: o que o compilador C# gera para `dynamic d = ...; d.Foo(1);`

Esboço conceitual com tipos reais da API (o código exato do Roslyn é mais elaborado):

```csharp
static CallSite<Action<CallSite, object, int>> site;
if (site == null)
    site = CallSite<Action<CallSite, object, int>>.Create(
        Binder.InvokeMember(CSharpBinderFlags.ResultDiscarded, "Foo", null, typeof(Program),
            new[] { CSharpArgumentInfo.Create(CSharpArgumentInfoFlags.None, null),
                    CSharpArgumentInfo.Create(CSharpArgumentInfoFlags.Constant, null) }));
site.Target(site, d, 1);
```

`Binder` está em `Microsoft.CSharp/.../Binder.cs`; `CallSite<T>.Create` em `CallSite.cs`; `site.Target` é o cache L0 (um delegate).

### Anatomia de uma regra

```text
regra (delegate T) =
   if (restrição(argumentos))        // ex.: arg0.GetType() == Foo
        return corpo(argumentos)      // ex.: ((Foo)arg0).Foo(1)
   else return site.Update(...)       // miss: volta ao UpdateAndExecute
```

`BindingRestrictions.cs` combina restrições de tipo, instância e expressão arbitrária. O resultado final é uma `Expression` compilada pelo `LambdaCompiler` (arquivo 07).

### Fluxo de `UpdateAndExecute` (conforme comentários de `CallSite.cs`)

1. Tenta as regras do L1 (array do site, até 10).
2. Em miss, consulta o L2 (`RuleCache` do binder, até 128; regra nova é inserida em `MaxRules / 2`).
3. Em miss total, chama o `CallSiteBinder.Bind(...)`, que consulta o objeto (`IDynamicMetaObjectProvider`) ou cai no `Fallback` da linguagem.
4. A nova regra é compilada e vira o novo `Target` (L0).

### `ExpandoObject`

Existe `ExpandoClass.cs` junto de `ExpandoObject.cs`, indicando que as propriedades são descritas por uma "classe" separada do objeto (ideia semelhante a *hidden classes*). Os detalhes de implementação não foram lidos aqui; trate esta frase como inferência a partir da estrutura de arquivos.

### Quando o dinâmico é a ferramenta certa

- Interop com COM, planilhas, objetos de script.
- Consumo de JSON/YAML sem tipos.
- Duck typing pontual sem interface comum.
- Não é apropriado para caminhos quentes, bibliotecas públicas tipadas ou AOT-first.

### Custo (ordem de grandeza qualitativa)

| Situação | Custo |
|---|---|
| 1a execução de um site | binding + `Compile` da regra |
| site monomórfico em regime | teste + chamada de delegate |
| site polimórfico (até 10 formas) | varre L1 |
| site megamórfico (>10) | recai em L2/binding com frequência |

### Protocolo do meta-objeto (resumo)

Um `DynamicMetaObject` conhece: valor, `Expression` que o representa e `BindingRestrictions`. Cada operação (`BindGetMember`, `BindInvokeMember`, `BindBinaryOperation` ...) devolve outro `DynamicMetaObject` com a expressão do resultado e as restrições. Os nomes dos binders (`GetMemberBinder`, `InvokeMemberBinder` ...) estão em `System/Dynamic/`; os nomes dos métodos `Bind*` seguem a API pública (conhecimento geral).

### Relação com os outros arquivos

- 07: as regras são `Expression` compiladas.
- 06: operadores dinâmicos usam `CSharpBinaryOperationBinder`.
- 08: o fallback do binder C# usa reflexão para resolver membros.
- 01: `CallSite<T>.Target` é um delegate; o L0 troca esse delegate.


### Glossário

| Termo | Significado |
|---|---|
| Call site | ponto de chamada dinâmico que guarda caches |
| Binder | componente que resolve uma operação dinâmica |
| Regra | par (restrição, ação) compilado |
| Restrição | condição que mantém a regra válida |
| Fallback | resolução padrão da linguagem quando o objeto não decide |

### Testes sugeridos

1. Chamar o mesmo site com dois tipos alternando, e depois com 11 tipos, observando recompilações.
2. `ExpandoObject` com propriedade adicionada depois da primeira chamada.
3. Operador `+` dinâmico com `int`, `string` e tipo de usuário.
4. Exceção de binding (membro inexistente) e a mensagem resultante.
5. Execução em plataforma sem geração de código (interpretador).

## Por quê

- **Interop com COM, JSON, linguagens dinâmicas, ORMs**: o tipo é desconhecido no compilador.
- **Custo amortizado**: o despacho caro (reflexão + análise) acontece uma vez por forma de operandos, depois um teste + delegate compilado.
- **Extensibilidade**: o protocolo `DynamicMetaObject` deixa cada objeto/linguagem decidir.
- **Unificação**: operações dinâmicas viram árvores de expressão, reusando o compilador de IL/interpretador existentes.

## Trade-offs

- Primeira chamada é muito lenta (binding + `Compile`); sites polimórficos além de 10 regras reciclam o L1.
- Memória: delegates compilados por site e regras nos caches; leaks possíveis com tipos colecionáveis.
- Perde checagem de tipo em tempo de compilação, refatoração e análise estática.
- Não funciona (ou funciona só interpretado) em AOT/trimmed sem geração de código; `Microsoft.CSharp` depende de reflexão.
- Mensagens de erro em runtime (`RuntimeBinderException`) em vez de compile-time.
- Dependência de `System.Linq.Expressions` e de um binder por linguagem.

## Como aplicar na sua linguagem

1. Separe **call site** (cache + ponto de chamada) de **binder** (política de resolução). O site é genérico; o binder é por linguagem/objeto.
2. Modele o cache em níveis: um monomórfico no site, um polimórfico pequeno por site, um compartilhado por binder. Os limites do .NET (10 e 128) são ponto de partida, não verdade absoluta.
3. Cada regra = **(guarda, ação)**. A guarda deve ser barata (teste de tipo/versão de forma).
4. Defina o protocolo de objetos que se auto-despacham (um `IDynamicMetaObjectProvider` mínimo) antes de empilhar `Expando`/`DynamicObject`.
5. Faça o JIT/interpretador da sua IR compilar regras; evita uma segunda infraestrutura para o dinâmico.
6. Documente claramente o custo da primeira chamada e forneça modo estático equivalente (ex.: `cast` + chamada direta).
7. Se sua linguagem tem tipagem gradual, decida onde o `dynamic` "contamina" resultados (em C#, o resultado é `dynamic`).

## Referências

- `src/libraries/System.Linq.Expressions/src/System/Runtime/CompilerServices/CallSite.cs`, `CallSiteBinder.cs`, `RuleCache.cs`
- `src/libraries/System.Linq.Expressions/src/System/Dynamic/` (binders, `DynamicMetaObject.cs`, `ExpandoObject.cs`, `DynamicObject.cs`, `BindingRestrictions.cs`)
- `src/libraries/System.Linq.Expressions/src/System/Linq/Expressions/DynamicExpression.cs`
- `src/libraries/Microsoft.CSharp/src/Microsoft/CSharp/RuntimeBinder/`
