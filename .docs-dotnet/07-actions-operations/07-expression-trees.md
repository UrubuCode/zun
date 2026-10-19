# Expression trees (código como dado)

## Mecanismo no .NET

`System.Linq.Expressions` representa código como uma **árvore de objetos imutáveis** que pode ser inspecionada, transformada e, opcionalmente, compilada para um delegate. O compilador C# transforma uma lambda atribuída a `Expression<Func<...>>` em chamadas às fábricas `Expression.*` (isso é comportamento do compilador, conhecimento geral); o runtime não tem opcode para isso.

Código: `src/libraries/System.Linq.Expressions/src/System/Linq/Expressions/`.

### Modelo de nós

- Base: `Expression.cs`, enum de tipos de nó `ExpressionType.cs` (inclui `Quote`, que representa "uma expressão com valor constante do tipo Expression" que pode referenciar parâmetros do contexto — comentário em `ExpressionType.cs` ~linha 172).
- Nós concretos, um arquivo cada: `BinaryExpression`, `UnaryExpression`, `ConditionalExpression`, `ConstantExpression`, `ParameterExpression`, `MethodCallExpression`, `MemberExpression`, `NewExpression`, `NewArrayExpression`, `LambdaExpression`, `InvocationExpression`, `BlockExpression`, `LoopExpression`, `GotoExpression`/`LabelExpression`/`LabelTarget`, `SwitchExpression`, `TryExpression`/`CatchBlock`, `IndexExpression`, `TypeBinaryExpression`, `MemberInitExpression`, `ListInitExpression`, `DefaultExpression`, `DebugInfoExpression`, `RuntimeVariablesExpression`, `DynamicExpression`.
- Ou seja: não só expressões, mas **statements, loops, goto, try/catch, blocos com variáveis** — a API cobre "DLR trees" (DLR = Dynamic Language Runtime).
- Visitação: `ExpressionVisitor.cs` (reescreve árvores imutáveis, devolvendo nós novos só quando um filho muda), `DynamicExpressionVisitor.cs`.
- Depuração: `DebugViewWriter.cs`, `ExpressionStringBuilder.cs`.
- Operadores definidos pelo usuário são resolvidos na criação do nó: `BinaryExpression.GetUserDefinedBinaryOperator` (arquivo 06).

### Compilação para delegate

`LambdaExpression.Compile()` (`LambdaExpression.cs`, ~linha 137) chama `Compiler.LambdaCompiler.Compile(this)`. Existem **dois backends**:

1. **Compilador para IL** (`Compiler/LambdaCompiler*.cs`, 29 arquivos): cria um `DynamicMethod` (`LambdaCompiler.cs` ~linha 74: `new DynamicMethod(lambda.Name ?? "lambda_method"+n, lambda.ReturnType, parameterTypes, true)`), emite IL com `ILGen`, e cria o delegate. Constantes que não cabem em IL são mantidas num objeto `Closure` (`System/Runtime/CompilerServices/Closure.cs`), por isso `CanEmitBoundConstants => _method is DynamicMethod`.
2. **Interpretador** (`Interpreter/`, 45 arquivos: `AddInstruction`, `CallInstruction`...). `LambdaExpression.Compile(bool preferInterpretation)` usa `new Interpreter.LightCompiler().CompileTop(this).CreateDelegate()` quando `CanInterpret && preferInterpretation`. `Expression.CanCompileToIL => RuntimeFeature.IsDynamicCodeSupported` e `CanInterpret => true` (~linhas 29-32): em plataformas sem geração de código dinâmica (AOT, WASM), o interpretador é o fallback.

Passes do compilador de IL (pasta `Compiler/`):

- `VariableBinder` e `CompilerScope`/`HoistedLocals`: determinam quais variáveis são capturadas por lambdas aninhadas e as içam para o closure.
- `StackSpiller`: reescreve a árvore para que expressões com efeitos colaterais que exigem pilha vazia (try/catch dentro de operando, por exemplo) funcionem; IL exige pilha vazia em entrada de `try`.
- `LambdaCompiler.ControlFlow`, `.Statements`, `.Logical`, `.Unary`, `.Binary` geram os respectivos IL.
- `AnalyzedTree`, `BoundConstants`.

### Dados como código: Queryable

`src/libraries/System.Linq.Queryable/src/System/Linq/`: `EnumerableQuery`, `EnumerableRewriter`, `Queryable.cs`. Um `IQueryable` monta uma árvore `Expression` por chamada de operador; um *provider* (ex. SQL) lê a árvore e traduz. `EnumerableRewriter` reescreve a árvore para `Enumerable` (LINQ to Objects).

### Dinâmico

`DynamicExpression` + `CallSite<T>` conectam árvores ao DLR (arquivo 09).


### Exemplo mínimo (C# ilustrativo, API real)

```csharp
ParameterExpression x = Expression.Parameter(typeof(int), "x");
Expression<Func<int,int>> e =
    Expression.Lambda<Func<int,int>>(
        Expression.Add(x, Expression.Constant(1)), x);

Func<int,int> f = e.Compile();   // IL via DynamicMethod, ou interpretador
int y = f(41);                   // 42
```

`Compile()` segue `LambdaExpression.Compile()` (`LambdaExpression.cs` ~137): `CanCompileToIL` decide entre `LambdaCompiler.Compile(this)` e `Interpreter.LightCompiler`.

### Pipeline do compilador de IL (fases)

```text
LambdaExpression
  -> VariableBinder         (descobre escopos e variáveis capturadas)
  -> AnalyzedTree           (guarda escopos e metadados)
  -> StackSpiller           (reescreve para pilha vazia em try/catch etc.)
  -> LambdaCompiler.Emit*   (ILGen -> DynamicMethod)
  -> CreateDelegate         (com Closure como 1o argumento)
```

Os arquivos de cada fase existem em `Compiler/`; a ordem exata é a lógica de `LambdaCompiler` e não foi conferida linha a linha.

### Interpretador (modo sem JIT)

A pasta `Interpreter/` implementa uma máquina de pilha própria com uma classe por instrução (`AddInstruction`, `AndInstruction`, `CallInstruction`, `BranchLabel`...). O `LightCompiler` traduz a árvore em instruções e cria um delegate que roda o interpretador. É o que mantém `Compile()` funcional onde `RuntimeFeature.IsDynamicCodeSupported` é falso.

### Imutabilidade e reescrita

`ExpressionVisitor` visita filhos e cria um novo nó apenas se algum filho mudou, permitindo transformações (substituir parâmetros, expandir sub-árvores) sem mutação. `EnumerableRewriter` (`System.Linq.Queryable`) é um exemplo real de visitor que troca chamadas `Queryable.*` por `Enumerable.*`.

### Limites práticos

| Aspecto | Observação |
|---|---|
| Custo de `Compile()` | alto; cachear o delegate |
| Constantes não embutíveis | vão para `Closure` (`CanEmitBoundConstants`) |
| Lambdas aninhadas | closures com escopos içados (`HoistedLocals`) |
| Depuração | `DebugViewWriter.cs` imprime a árvore |
| Visibilidade | `DynamicMethod` criado com `true` no último parâmetro (`skipVisibility`) |

### Relação com os outros arquivos desta pasta

- Delegates (01): `Compile()` devolve um delegate.
- Operadores (06): resolução por nome `op_*`.
- Reflexão (08): nós `MethodCallExpression` guardam `MethodInfo`.
- dynamic (09): `DynamicExpression` e call sites usam árvores para suas regras.

### Catálogo de nós por finalidade

| Finalidade | Nós |
|---|---|
| Valores | `ConstantExpression`, `ParameterExpression`, `DefaultExpression` |
| Aritmética/lógica | `BinaryExpression`, `UnaryExpression`, `ConditionalExpression` |
| Acesso | `MemberExpression`, `IndexExpression` |
| Chamada | `MethodCallExpression`, `InvocationExpression`, `NewExpression`, `DynamicExpression` |
| Inicialização | `MemberInitExpression`, `ListInitExpression`, `NewArrayExpression` |
| Controle | `BlockExpression`, `LoopExpression`, `GotoExpression`, `LabelExpression`, `SwitchExpression`, `TryExpression` |
| Tipo | `TypeBinaryExpression` |
| Funções | `LambdaExpression` |
| Meta | `RuntimeVariablesExpression`, `DebugInfoExpression` |


### Glossário

| Termo | Significado |
|---|---|
| Homoiconicidade | código e dados com a mesma representação |
| Quote | embutir uma árvore dentro de outra como valor |
| Hoisting | mover variável capturada para um objeto de ambiente |
| Spilling | salvar valores temporários em locais para liberar a pilha |
| DLR | Dynamic Language Runtime, camada sobre árvores e call sites |

### Testes sugeridos

1. Compilar e interpretar a mesma árvore e comparar resultados.
2. Lambda aninhada capturando variável de escopo externo e alterando-a.
3. `try/catch` dentro de argumento de chamada (exige spilling).
4. Visitor que troca um parâmetro por constante e confirmar que sub-árvores sem mudança são reaproveitadas.
5. Serializar a árvore para texto e comparar com a saída de `DebugView`.

## Por quê

- **Homoiconicidade parcial**: o programa se torna dado para análise (provedores de consulta, tradução para SQL, mapeadores, regras).
- **Geração de código em runtime sem escrever IL à mão**: tipos checados, escopo léxico, closures e fluxo de controle de alto nível; o compilador cuida de IL, hoisting e spilling.
- **Linguagens dinâmicas** (IronPython, etc.) usam árvores como IR alvo.
- **Interpretador** como segunda via mantém funcionalidade onde não há JIT.

## Trade-offs

- A árvore é uma API de construção, não sintaxe: criar árvores à mão é verboso e erros aparecem na criação do nó (validação eager) ou na compilação.
- `Compile()` é caro (emite IL, cria `DynamicMethod`, aciona o JIT na primeira chamada); resultado deve ser **cacheado**.
- Delegates compilados são mais lentos que métodos estáticos em chamadas fechadas por causa do `Closure` (primeiro argumento) e de constantes via array.
- Subconjunto: nem toda lambda C# é representável (sem statements em lambdas de expressão, no compilador C#) — limitação da linguagem, não da biblioteca, que suporta blocos.
- Trimming/AOT: construção de árvores com métodos via reflection exige metadados preservados; `Compile` precisa de interpretador.
- Imutabilidade facilita raciocínio mas leva a reescrita via visitor.

## Como aplicar na sua linguagem

1. Se quiser metaprogramação, ofereça uma **AST de primeira classe** com nós tipados, imutável, com visitor que preserva identidade quando nada muda.
2. Cubra statements e fluxo de controle (não só expressões), ou sua AST não servirá de IR para linguagens dinâmicas.
3. Faça a validação de tipo na **construção do nó** e erros com mensagens centralizadas (arquivo `Strings.cs`/`Error.cs` do repositório).
4. Forneça dois backends: compilador nativo/JIT e **interpretador**; selecione por `CanCompile`/flag de plataforma.
5. Resolva captura de variáveis em um passe separado (binding -> hoisting) e padronize a representação do closure.
6. Se o seu backend exige pilha vazia em certos pontos (try, exceções), inclua um passe equivalente ao `StackSpiller`.
7. Defina como "quote" funciona (nó `Quote`): embutir uma árvore dentro de outra preservando parâmetros livres.
8. Dê uma forma legível de imprimir (`DebugView`) desde cedo: é essencial para depurar transformações.

## Referências

- `src/libraries/System.Linq.Expressions/src/System/Linq/Expressions/Expression.cs`, `ExpressionType.cs`, `ExpressionVisitor.cs`, `LambdaExpression.cs`
- `.../Linq/Expressions/Compiler/` (`LambdaCompiler.cs`, `StackSpiller.cs`, `VariableBinder.cs`, `CompilerScope.cs`, `ILGen.cs`)
- `.../Linq/Expressions/Interpreter/` (`LightCompiler` e instruções)
- `src/libraries/System.Linq.Expressions/src/System/Runtime/CompilerServices/Closure.cs`
- `src/libraries/System.Linq.Queryable/src/System/Linq/` (`Queryable.cs`, `EnumerableRewriter.cs`)
