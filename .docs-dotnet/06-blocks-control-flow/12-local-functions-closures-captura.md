# Local functions e closures (captura de variáveis)

Closures e local functions **não existem no runtime**. O CLR conhece apenas: métodos, classes, structs, campos, delegates e `ldftn`/`newobj`. Quando uma linguagem tem funções aninhadas que enxergam variáveis do escopo externo, o compilador faz **conversão de closure** (closure conversion / lambda lifting): as variáveis capturadas viram campos de um objeto de "ambiente", e a função aninhada vira um método comum que recebe esse ambiente como `this` ou como parâmetro. Este documento separa o que o runtime fornece (delegates, atributos, JIT) do que é convenção do compilador, usando as pistas deixadas nas ferramentas deste repositório.

## Mecanismo no .NET

### 1. Convenção de nomes gerados (evidência local)

O compilador C# não está neste repositório, mas o trimmer (ILLink) e o ILCompiler (NativeAOT) precisam **reconhecer** o código gerado para fazer análise de fluxo de dados. O arquivo `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs` documenta a convenção em comentários:

| Entidade gerada | Padrão de nome | Comentário no código |
|---|---|---|
| Classe de closure (instância) | `<>c__DisplayClass1_0` | "instance lambdas (which are emitted into a class like `<>c__DisplayClass1_0`)" |
| Classe de lambdas estáticas | `<>c` | "static lambdas (which are emitted into a class like `<>c`)" |
| Método de lambda | `<UserMethod>b__0_1` | "Lambda methods have generated names like `<UserMethod>b__0_1`" (detecção pelo caractere `b` após `>`) |
| Local function | `<UserMethod>g__LocalFunction\|0_1` | "Local functions have generated names like `<UserMethod>g__LocalFunction|0_1`" (caractere `g`) |
| State machine | `<Method>d__N` | veja docs 5 e 6 |

Todos começam com `<` (`IsGeneratedMemberName`: `memberName[0] == '<'`), caracter inválido em identificadores de linguagem, o que evita colisão. Esse truque é um contrato de fato entre o compilador e as ferramentas.

Os mesmos comentários em `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/Dataflow/CompilerGeneratedState.cs` (~linha 370-395) mostram o resultado da conversão com um exemplo:

```csharp
void M<T>() { Action a = () => { Console.WriteLine(typeof(T)); }; }
// O compilador gera:
class <>c__DisplayClass0<T> {
    public void <M>b__0() { Console.WriteLine(typeof(T)); }
}
```

O comentário explica que o parâmetro de tipo `T` do método **é reintroduzido como parâmetro de tipo da classe gerada** ("alpha renaming"): as ferramentas precisam mapear de volta `T` da classe gerada para `T` do método. Ou seja, closures sobre métodos genéricos tornam a classe de closure genérica.

### 2. A conversão (convenção do compilador; forma conhecida)

Para

```csharp
int Soma(int[] xs, int k) {
    int total = 0;
    foreach (var x in xs) Aplica(() => total += x * k);
    return total;
}
```

o esquema típico é:

1. identificar variáveis capturadas (`total`, `k`, e `x` por iteração);
2. para cada escopo que contém variáveis capturadas, criar uma instância da classe `<>c__DisplayClassN_M` **no ponto de entrada do escopo**; as variáveis capturadas passam a ser campos dessa instância (todos os acessos no método, inclusive os locais, usam o campo);
3. a lambda vira um método de instância da classe de closure (`<Soma>b__0`);
4. no ponto de uso, cria-se um delegate: `ldloc closure; ldftn <Soma>b__0; newobj Action::.ctor(object, native int)`.

O momento de criação do ambiente define a semântica: `x` de um `foreach` é novo a cada iteração, portanto o ambiente é criado dentro do laço; `total` e `k` vivem no ambiente do método. Essa regra é de linguagem.

**Local functions** seguem o mesmo esquema, com uma otimização importante: se a local function **não é convertida em delegate**, o compilador pode passar o ambiente como **struct por referência** (`ref`), evitando alocação. Os nomes `g__` no `CompilerGeneratedNames` indicam métodos gerados para local functions; que usem `struct` de closure é convenção do compilador (não verificável aqui).

### 3. O que o runtime fornece: delegates

`System.Delegate` / `MulticastDelegate` implementam a ligação (alvo, método). Em `src/coreclr/System.Private.CoreLib/src/System/Delegate.CoreCLR.cs` (linhas ~20-50) os campos comentados são:

- `_target`: "the object we will invoke on" (para closures, a instância de `<>c__DisplayClass`);
- `_methodPtr`: "a pointer to the method we will invoke. It could be a small thunk if this is a static or UM call";
- `_methodPtrAux`: para método estático passado como delegate, guarda o ponteiro real, e `_methodPtr` aponta para um pequeno thunk que remove o `this`;
- `_extraData`: contagem multicast, marcador de função não gerenciada ou `MethodDesc`;
- `_helperObject`: array de wrappers (multicast), cache de `MethodInfo` ou `LoaderAllocator` (delegates colecionáveis).

A propriedade `IsClosed => _methodPtrAux == 0` e `Target => ... IsClosed ? _target : null` mostram a distinção *delegate fechado* (tem alvo, por exemplo closure) vs. *aberto* (static). "Keep `_target` and `_methodPtr` next to each other for optimal delegate invoke performance."

Instruções de IL envolvidas (`opcode.def`): `ldftn` (linha 300), `ldvirtftn` (301) e `newobj` (152) com o construtor `(object, native int)` do delegate.

### 4. O JIT e delegates de closures

- `Compiler::fgOptimizeDelegateConstructor` (`src/coreclr/jit/flowgraph.cpp`, linha ~1055): "try and optimize construction of a delegate", chamado do importador de chamadas (`importercalls.cpp`, ~linha 1231) quando o padrão `ldftn` + `newobj` é reconhecido; recebe o token resolvido do método destino para gerar construção mais barata.
- GDV (guarded devirtualization) também se aplica a invocações de delegates (`DeabstractionAndConditionalEscapeAnalysis.md`, ~linha 736: "The JIT also does GDV on the delegate invocation").
- Alocação em pilha de closures/delegates que não escapam é um objetivo documentado em `docs/design/coreclr/jit/object-stack-allocation.md` (protótipo de "stack allocation of a simple delegate that's directly invoked"; "the delegate cases from the prototype" estão no roteiro). Ou seja, o custo de alocar a closure é reconhecido como um alvo de análise de escape, mas isso é otimização do JIT, não semântica.
- O inliner inlina o método da lambda se o delegate é conhecido e direto; veja `inline-size-estimates.md`, `inlining-plans.md`.

### 5. Captura e ciclo de vida

Como o ambiente é um objeto de heap, as variáveis capturadas **vivem enquanto o delegate viver**. Em termos de GC isso é uma raiz normal: o delegate referencia `_target` (a closure), que referencia os campos capturados. Para métodos `async`/iteradores (docs 5 e 6), a própria state machine é o ambiente: locais viram campos do tipo `<M>d__N`. Se o método tiver também lambdas capturando os mesmos locais, a closure e a state machine coexistem (a state machine mantém uma referência para o display class).

### 6. Captura e regras de segurança do runtime

Itens que **não podem** ser capturados porque não podem viver no heap:
- `ref`/byref locals e `ref struct` (por exemplo `Span<T>`): a regra do runtime é que referências interiores/ref-like não podem ser campos de classes. A spec de runtime-async também declara que byrefs e ref-like não são hoisted através de suspensão (`runtime-async.md`: "By-ref variables may not be hoisted across suspension points").
- Locais `pinned` (doc 11).

A proibição em C# para lambdas capturarem `ref` locals é consequência dessas regras; a implementação do erro é do compilador.

### 7. Trimming e AOT

Como o código é gerado e nomeado pelo compilador, ferramentas como ILLink e ILCompiler precisam rastrear que um método "vive" dentro de uma classe de closure para propagar anotações (por exemplo `DynamicallyAccessedMembers`) de um método para suas lambdas. O `CompilerGeneratedState.cs` implementa esse mapeamento, inclusive o "alpha renaming" de parâmetros de tipo genéricos. É uma evidência de que a conversão de closure cria um **acoplamento de metadados** entre o método original e o gerado.

## Por quê

1. **O runtime fica simples.** Nenhuma noção de "frame capturado" ou "função aninhada": delegates e objetos bastam, e os mesmos mecanismos (GC, vtables, inlining) funcionam.
2. **Semântica de captura por referência.** Compartilhar a variável entre a função externa e as internas exige que ela viva num local comum e mutável: o campo de um objeto de heap.
3. **Isolamento de frames.** Como o stack frame pode terminar antes do delegate, a variável precisa sair do frame ("escapar"). Conversão de closure resolve isso de forma uniforme.
4. **Otimização específica.** Local functions sem escapar do método podem evitar a alocação (struct passado por `ref`), o que é uma razão para distingui-las de lambdas.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Conversão de closure no compilador | Runtime não precisa de funções aninhadas | Alocação por closure; nomes gerados e acoplamento entre ferramentas (trimmer) |
| Variáveis capturadas viram campos (heap) | Semântica de captura por referência correta | Perda de enregistramento; acesso indireto; vida estendida (vazamento de memória involuntário) |
| Um ambiente por escopo | Semântica correta de iteração | Várias alocações aninhadas; múltiplos display classes encadeados |
| Delegate (`_target` + `_methodPtr`) | Chamada indireta uniforme | Custo de invocação indireta; delegates fechados alocam 1 objeto |
| Local functions sem delegate usam struct | Sem alocação | Código duplicado por tipo (genéricos sobre struct); regra de linguagem adicional |
| Nomes com `<` e `>` | Sem colisão | Contrato implícito para ferramentas; dependem do compilador |
| Captura de ref/ref-like proibida | Segurança de memória | Restringe expressividade (Span em lambdas) |
| Escape analysis do JIT | Elimina alocação quando possível | Roteiro/proto no `object-stack-allocation.md`; nem sempre aplicável |

## Como aplicar na sua linguagem

1. **Decida o modelo de função aninhada**: (a) conversão de closure em tempo de compilação (modelo .NET, sem suporte do runtime); (b) frames heap-allocated/static links; (c) display/environment passing no estilo funcional (OCaml, Lisp). Para um runtime com GC, (a) é pragmático.
2. **Faça análise de captura precisa** no front-end: variáveis que não escapam nunca viram campos. Marque, por variável: capturada? mutada depois da captura? vive além do escopo?
3. **Defina o ponto de criação do ambiente** pela regra de escopo da linguagem (`foreach`/laços criam um novo ambiente por iteração ou não). Documente isso: é uma fonte clássica de bugs (JavaScript `var` vs `let`).
4. **Distinga funções que escapam das que não escapam.** Para as que não escapam (chamadas diretas), passe o ambiente por referência/struct para evitar alocação (a distinção local function vs lambda do C#).
5. **Defina representação do callable**: par `(ambiente, ponteiro de código)` como o delegate do .NET (`_target`, `_methodPtr`). Escolha entre closure fechada e aberta; previna ponteiros soltos.
6. **Cuide de genéricos**: se funções genéricas capturam parâmetros de tipo, o ambiente deve ser genérico (alpha renaming: `<>c__DisplayClass0<T>`).
7. **Proíba capturar referências interiores/efêmeras** (byref, ref-like, pinned, ponteiros para a pilha), ou transforme-as de forma segura. Isso vale também para state machines (docs 5 e 6).
8. **Convenções de nomes e metadados gerados** devem ser estáveis e reconhecíveis (prefixo inválido para o usuário, como `<`), porque depuradores, trimmers e profilers dependem deles; planeje atributos como `CompilerGenerated` e mapeamentos para debug.
9. **Dê ao otimizador a chance de remover a alocação**: análise de escape, inlining de lambdas conhecidas, devirtualização de invocações de delegates.
10. **Considere o interplay com `async` e iteradores**: ambos transformam locais em campos; decida se o ambiente da lambda e da state machine são o mesmo objeto ou objetos encadeados.

## Apêndice: exemplo do ciclo completo (ilustrativo)

Fonte:

```csharp
Func<int> Contador() {
    int n = 0;
    return () => ++n;       // captura n por referência
}
```

Resultado conceitual após a conversão de closure:

```csharp
class <>c__DisplayClass0_0 {
    public int n;
    public int <Contador>b__0() { return ++n; }
}
Func<int> Contador() {
    var c = new <>c__DisplayClass0_0();
    c.n = 0;
    return new Func<int>(c.<Contador>b__0);   // ldloc c; ldftn ...; newobj Func<int>::.ctor
}
```

Estado na memória após `var f = Contador();`:

| Objeto | Campos relevantes |
|---|---|
| `Func<int>` (delegate) | `_target` = a closure, `_methodPtr` = código de `<Contador>b__0` |
| `<>c__DisplayClass0_0` | `n` (o contador, vivo enquanto o delegate viver) |

Chamar `f()` duas vezes retorna 1 e 2: o `n` é compartilhado, porque vive no objeto, não no frame de `Contador` (já encerrado). Se o contador fosse capturado por duas lambdas, ambas compartilhariam a mesma instância de display class.

## Apêndice: perguntas de design

1. **Captura por valor ou por referência?** O C# captura **variáveis** (por referência ao ambiente); Java exige variáveis efetivamente finais (captura por valor). A escolha define se é preciso heap para as variáveis.
2. **Quando o ambiente é criado?** No início do escopo que declara a variável. Decida a regra para laços.
3. **Um ambiente por escopo ou um ambiente achatado?** O C# usa uma classe por escopo com variáveis capturadas e encadeia ambientes quando há aninhamento; alternativas compartilham um único registro por função.
4. **Funções locais não escapantes:** passe o ambiente por referência a uma struct para evitar a alocação.
5. **Representação do callable:** `(alvo, código)`, ou objeto com método `Invoke` (functor).
6. **Genéricos e inferência de tipo do ambiente:** parâmetros de tipo do método tornam-se parâmetros do ambiente.
7. **Identidade e igualdade de delegates:** o .NET compara `_target` e `_methodPtr`; lambdas estáticas são cacheadas em campos da classe `<>c` (comentário em `CompilerGeneratedNames`: "static lambdas ... emitted into a class like `<>c`"). Não confirmado aqui que há cache; trate como convenção do compilador.
8. **Metadados para ferramentas:** nomes reconhecíveis, atributo de gerado pelo compilador, mapeamento método-fonte para método-gerado.
9. **Segurança de memória:** não capturar ref/ref-like/pinned.
10. **Otimização:** inlining do corpo, devirtualização de invocação, análise de escape para alocar o ambiente na pilha (roteiro em `object-stack-allocation.md`).

## Referências

- `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs` (convenções `<>c__DisplayClass`, `<>c`, `b__`, `g__`, `d__`, `<>2__current`).
- `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/Dataflow/CompilerGeneratedState.cs` (mapeamento de tipos gerados, exemplo de alpha renaming de `T`, ~370-395), `FlowAnnotations.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/Delegate.CoreCLR.cs` (campos `_target`, `_methodPtr`, `_methodPtrAux`, `_extraData`, `_helperObject`; `IsClosed`, `Target`).
- `src/libraries/System.Private.CoreLib/src/System/Delegate.cs`, `MulticastDelegate.cs`.
- `src/coreclr/inc/opcode.def` (`ldftn`, `ldvirtftn`, `newobj`).
- `src/coreclr/jit/flowgraph.cpp` (`fgOptimizeDelegateConstructor`, ~1043), `src/coreclr/jit/importercalls.cpp` (~1231).
- `docs/design/coreclr/jit/object-stack-allocation.md`, `DeabstractionAndConditionalEscapeAnalysis.md`, `inline-size-estimates.md`, `inlining-plans.md`.
- `docs/design/specs/runtime-async.md` (byref e ref-like não hoisted).

> Nota de confiança: a forma exata do IL e a estratégia de criação de ambientes (por escopo, uso de struct para local functions) são convenções do compilador C#. O que está confirmado aqui: a convenção de nomes (via ILLink/ILCompiler), a estrutura de `Delegate` e as otimizações do JIT para delegates.
