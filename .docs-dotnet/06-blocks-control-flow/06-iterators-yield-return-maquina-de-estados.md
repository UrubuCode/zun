# Iterators (yield return) — a máquina de estados gerada

`yield return` / `yield break` transformam um método em uma **corrotina assimétrica pull**: o consumidor chama `MoveNext()`, a produção retoma de onde parou, produz um valor em `Current` e suspende de novo. O .NET implementa isso **inteiramente no compilador de linguagem**: o runtime/JIT não conhece `yield`. O que existe no runtime são as interfaces (`IEnumerable<T>`, `IEnumerator<T>`, `IAsyncEnumerable<T>`), atributos marcadores e o builder dos iteradores assíncronos. A boa notícia para quem projeta linguagem: o repositório contém, em `System.Linq`, **iteradores escritos à mão** que mostram exatamente a forma da máquina de estados.

## Mecanismo no .NET

### 1. O contrato no runtime

- `IEnumerable<T>`, `IEnumerator<T>` (`src/libraries/System.Private.CoreLib/src/System/Collections/Generic/IEnumerable.cs`, `IEnumerator.cs`): `GetEnumerator()`, `MoveNext()`, `Current`, `Dispose()`.
- `IAsyncEnumerable<T>` e `IAsyncEnumerator<T>` (`.../Collections/Generic/IAsyncEnumerable.cs`, `IAsyncEnumerator.cs`): `IAsyncEnumerator<out T> : IAsyncDisposable` com `ValueTask<bool> MoveNextAsync()` e `T Current { get; }`.
- Atributos marcadores: `IteratorStateMachineAttribute` (`.../Runtime/CompilerServices/IteratorStateMachineAttribute.cs`) e `AsyncIteratorStateMachineAttribute`, ambos derivados de `StateMachineAttribute` (`StateMachineAttribute.cs`), que guarda `Type StateMachineType`. Servem para ferramentas (depurador, trimmer) associarem o método original ao tipo gerado.

### 2. O que o compilador gera (convenção; não está neste repo)

O compilador C# transforma um método `yield` em:
- um **tipo aninhado de máquina de estados** com nome no padrão `<NomeDoMétodo>d__N` (confirmado como convenção pelos reconhecedores de nome em `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs`);
- o campo do valor corrente com nome `<>2__current` (o mesmo arquivo trata isso: "Current field is `<>2__current`", função `IsStateMachineCurrentField`);
- o método original reduzido a um stub que instancia a máquina e a devolve como `IEnumerable<T>`.

O tipo gerado implementa **ao mesmo tempo** `IEnumerable<T>` e `IEnumerator<T>` (o mesmo objeto serve de enumerável e de enumerador na primeira chamada, para evitar uma alocação). Isso é dedutível do que a própria biblioteca faz à mão (próxima seção) e é a técnica conhecida do compilador, mas o código exato do Roslyn não está aqui.

### 3. Prova viva: `Iterator<TSource>` do LINQ (escrito à mão)

`src/libraries/System.Linq/src/System/Linq/Iterator.cs` define:

```csharp
private abstract partial class Iterator<TSource> : IEnumerable<TSource>, IEnumerator<TSource>
{
    private readonly int _threadId = Environment.CurrentManagedThreadId;
    private protected int _state;
    private protected TSource _current = default!;

    public TSource Current => _current;
    private protected abstract Iterator<TSource> Clone();

    public virtual void Dispose() { _current = default!; _state = -1; }

    public Iterator<TSource> GetEnumerator()
    {
        Iterator<TSource> enumerator = _state == 0 && _threadId == Environment.CurrentManagedThreadId ? this : Clone();
        enumerator._state = 1;
        return enumerator;
    }

    public abstract bool MoveNext();
    ...
}
```

Lições embutidas no código e nos comentários:

- **Estado inteiro**: `_state == 0` significa "ainda não enumerado"; `1` é o primeiro `MoveNext`; `-1` é "terminado/descartado".
- **Objeto duplo (enumerável e enumerador)**: o primeiro `GetEnumerator()` na mesma thread que criou devolve `this`; chamadas posteriores ou de outra thread devolvem `Clone()`. Motivação (pelo comentário): "an iterator also serves as its own enumerator, so the state of an iterator may change as it is being enumerated". Por isso o comentário manda guardar o que define o *valor* do iterador em campos `readonly` e o que é estado de *enumeração* em campos mutáveis.
- **`Dispose()` zera `_current` e marca `-1`**.

Um `MoveNext` concreto (`IEnumerableWhereIterator<TSource>`, `src/libraries/System.Linq/src/System/Linq/Where.cs`, ~linha 121):

```csharp
public override bool MoveNext()
{
    switch (_state)
    {
        case 1:
            _enumerator = _source.GetEnumerator();
            _state = 2;
            goto case 2;
        case 2:
            while (_enumerator.MoveNext())
            {
                TSource item = _enumerator.Current;
                if (_predicate(item)) { _current = item; return true; }
            }
            Dispose();
            break;
    }
    return false;
}
```

Isto é, literalmente, a forma de uma máquina de estados de `yield`:
1. `switch (_state)` no topo seleciona o ponto de retomada;
2. variáveis que sobrevivem entre chamadas (`_enumerator`, `_current`) são **campos**;
3. `yield return x` vira `_current = x; _state = N; return true;`;
4. fim do laço vira `Dispose(); return false;`.

Outra variante em arrays (~linha 175) usa o próprio `_state` como índice (`int index = _state - 1; ... index = _state++`), uma otimização de armazenamento específica.

### 4. `try/finally` dentro de iteradores

Quando o corpo do iterador tem `try/finally`, a máquina precisa executar o `finally` em dois cenários: ao sair normalmente do bloco e quando o consumidor chama `Dispose()` no meio da enumeração. A regra de projeto é exigir que o `Dispose()` do enumerador execute os `finally` pendentes do estado atual. Como a máquina de estados não pode usar regiões IL de `try` que atravessem suspensões (o `MoveNext` retorna no meio do bloco), o compilador converte os `try/finally` em lógica por estado (convenção do compilador; não está neste repositório). Já a **biblioteca** mostra o lado do consumidor: `Dispose()` do `Iterator<T>` encerra o enumerador interno.

Observação confirmada no runtime: uma região protegida em IL é um **intervalo contíguo de offsets** (doc 2) e `leave`/`endfinally` são as únicas formas de sair. Um método `MoveNext` que retorna no meio de um `try` não seria um "suspender e retomar" válido dentro do mesmo frame; por isso o estado precisa ser reificado em campos e o fluxo de controle original reconstruído por `switch`.

### 5. Iteradores assíncronos (`async` + `yield`)

Existe uma terceira família: métodos `async IAsyncEnumerable<T>`. O runtime fornece:
- `AsyncIteratorMethodBuilder` (`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/AsyncIteratorMethodBuilder.cs`): struct com `Create()`, `MoveNext<TSM>(ref TSM)` (que delega em `AsyncMethodBuilderCore.Start`), `AwaitOnCompleted`/`AwaitUnsafeOnCompleted` (que reutilizam `AsyncTaskMethodBuilder<VoidTaskResult>`) e `Complete()`. O comentário do campo `m_task`: "This will be the async state machine box created for the compiler-generated class (not struct) state machine"; ou seja, a máquina de iterador assíncrono é uma **classe** (porque precisa viver como `IAsyncEnumerator`).
- `ManualResetValueTaskSourceCore<T>` (`src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Sources/ManualResetValueTaskSourceCore.cs`) e `IValueTaskSource` (`IValueTaskSource.cs`): blocos de construção para implementar `ValueTask<bool> MoveNextAsync()` sem alocar uma `Task` por item. Que o código gerado os use é convenção do compilador.
- `ConfiguredCancelableAsyncEnumerable` (`.../CompilerServices/ConfiguredCancelableAsyncEnumerable.cs`) para `ConfigureAwait`/`WithCancellation`.

### 6. Interação com runtime-async

A spec de runtime-async (`docs/design/specs/runtime-async.md`) restringe os tipos de retorno a `Task`, `Task<T>`, `ValueTask`, `ValueTask<T>`. **Iteradores e iteradores assíncronos não estão entre eles**, então não são transformados pelo JIT; continuam sendo máquinas geradas pelo compilador (o iterador assíncrono pode, por dentro, usar `await` de métodos runtime-async, mas a máquina em si é do compilador). Esta inferência vem da lista de tipos de retorno permitidos na spec.

## Por quê

1. **Preguiça (laziness) e composição.** Consumidores fazem pull; produtores podem ser infinitos; cadeias LINQ compõem iteradores sem materializar listas.
2. **Sem suporte de runtime.** O .NET evitou stacks separadas ou continuações de primeira classe no CLR; a máquina de estados traduz uma corrotina em um objeto com `MoveNext`, que roda no stack normal. Isso mantém GC, debugger, stack walking e EH iguais ao resto do runtime.
3. **Custo previsível.** Uma alocação por enumeração (um objeto), mais zero ou uma clonagem.
4. **Interoperabilidade com bibliotecas.** Qualquer linguagem que gere `IEnumerable<T>` com `MoveNext` interopera com `foreach`, LINQ, etc.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Corrotina como máquina de estados | Sem stack switching; GC/EH inalterados | Código gerado volumoso; locais viram campos; `try/finally` com `yield` precisa de reescrita complexa |
| Objeto = enumerável + enumerador | Uma alocação no caso comum | Detalhes sutis (thread id, `Clone`) para evitar compartilhamento incorreto |
| Protocolo `MoveNext`/`Current` | Simples, universal | Dois membros por item (virtual/interface), sem fusão; depende de devirtualização/PGO |
| Estado `int` | Compacto, `switch` rápido | Ramificação via `switch` em todo `MoveNext`; IL com `goto case`/saltos |
| `Dispose` obrigatório para limpeza | Finalização determinística de `finally` | Consumidor que não descarta deixa recursos pendentes |
| Iteradores assíncronos como classe | Vive no heap, pode ser compartilhado | Mais alocação que métodos async simples |
| Iteradores fora do runtime-async | Runtime simples | Não ganham as otimizações do JIT (liveness, sem alocação) |

## Como aplicar na sua linguagem

1. **Escolha o modelo de corrotina antes do resto.**
   - *Stackless via máquina de estados* (modelo .NET): implementável só no front-end, sem runtime especial.
   - *Stackful* (fibras/green threads): exige runtime, mas elimina "cor" de função e reescrita de `try/finally`.
   Se for stackless, planeje a transformação como passo de lowering bem definido.
2. **Defina a representação do estado**: um inteiro (`0` inicial, `-1` terminado, `N` pontos de suspensão), campos para cada variável viva entre suspensões, e campo para o valor corrente.
3. **Escreva a transformação em três partes**: (a) hoisting de locals vivos a campos; (b) `switch(state)` de despacho no topo de `MoveNext`; (c) cada `yield` vira `current = v; state = k; return true;` seguido de um rótulo de retomada.
4. **Decida a semântica de `finally`/`defer` com `yield`**: o consumidor descartando o iterador deve executar os `finally` pendentes? O .NET diz que sim, via `Dispose`. Isso implica gerar uma tabela "estado → blocos finally a executar".
5. **Projete clonagem/reuso** para evitar dupla enumeração incorreta (o `_threadId` e `Clone()` do LINQ são o modelo).
6. **Defina o protocolo de fim**: `MoveNext` retorna `false` após `Dispose` interno; `Current` deve ser resetado para liberar referências (veja `Dispose` zerando `_current`).
7. **Seja explícito sobre mutabilidade de captura**: parâmetros do método original devem ser copiados para a máquina; a máquina é consumida uma vez por `GetEnumerator`.
8. **Para iteradores assíncronos**, separe o "próximo item" assíncrono do descarte assíncrono (`MoveNextAsync` + `DisposeAsync`), e use um tipo de promessa reutilizável (equivalente a `ManualResetValueTaskSourceCore`) para não alocar por item.
9. **Ofereça metadados de depuração**: um atributo (equivalente a `IteratorStateMachineAttribute`) ligando o método original ao tipo gerado, e nomes de campos reconhecíveis (o trimmer do .NET depende de reconhecer `<M>d__N` e `<>2__current`).

## Apêndice: esqueleto conceitual de um iterador gerado

```csharp
IEnumerable<int> Pares(int max) { for (int i = 0; i < max; i += 2) yield return i; }
```

Forma típica da máquina gerada (nomes e detalhes são convenção do compilador; o padrão estrutural é o mesmo que `Where.cs` mostra à mão):

```csharp
class Pares_d__0 : IEnumerable<int>, IEnumerator<int> {
    int _state;            // -2 = criado como enumerável; 0 = pronto para iniciar; N = retomar após yield N; -1 = terminado
    int _current;          // <>2__current
    int _initialThreadId;  // para decidir entre reutilizar this e clonar
    int max, i;            // parâmetro e local hoisted

    public bool MoveNext() {
        switch (_state) {
            case 0: _state = -1; i = 0; goto case_test;
            case 1: _state = -1; i += 2;
            case_test:
                if (i < max) { _current = i; _state = 1; return true; }
                break;
        }
        return false;
    }
    public int Current => _current;
    public IEnumerator<int> GetEnumerator() {
        if (_state == -2 && _initialThreadId == Environment.CurrentManagedThreadId) { _state = 0; return this; }
        var c = new Pares_d__0(0) { max = max };  // nova instância a partir do parâmetro original
        return c;
    }
    public void Dispose() { }   // sem finally pendente neste exemplo
}
```

Pontos a observar (todos ecoados em `Iterator<TSource>` do LINQ):
- `_state` com valores especiais para "não iniciado", "terminado" e pontos de retomada;
- a decisão `this` vs. nova instância baseada no thread id inicial (em `Iterator.cs`: `_state == 0 && _threadId == Environment.CurrentManagedThreadId ? this : Clone()`);
- locais vivos entre `yield`s (`i`, `max`) como campos.

## Apêndice: perguntas de design

1. O iterador é uma **sequência reiniciável** (`IEnumerable`) ou de uso único (`IEnumerator`)? O .NET unifica os dois no mesmo objeto para economizar uma alocação.
2. Quem garante isolamento entre enumerações simultâneas? Clonagem por `GetEnumerator`.
3. Como se encerra? `MoveNext` retorna `false` e `Dispose` libera recursos; consumidores que param cedo devem chamar `Dispose` (o `foreach` do C# faz isso via `using`, convenção).
4. O que é `Current` antes do primeiro `MoveNext` ou depois do fim? Indefinido no contrato; o `Dispose()` do LINQ zera `_current`.
5. `yield` dentro de `try`/`catch`? A linguagem C# proíbe `yield return` em `try` com `catch`, e permite em `try/finally` (restrição de linguagem; fora do runtime). Documente a sua.
6. `yield` dentro de lambdas? Em geral não; a lambda é outro método.
7. Iteração assíncrona: ver `AsyncIteratorMethodBuilder` e `IAsyncEnumerable`; o `await` e o `yield` compartilham a mesma máquina de estados.
8. Interface com o JIT: nada; mas métodos pequenos `MoveNext` e `Current` favorecem devirtualização/PGO. O `DeabstractionAndConditionalEscapeAnalysis.md` do JIT trata especificamente de remover a alocação e o custo de enumeradores via análise de escape.

## Referências

- `src/libraries/System.Linq/src/System/Linq/Iterator.cs` (classe base `Iterator<TSource>`), `Where.cs` (`MoveNext` por estado), `Range.SpeedOpt.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Collections/Generic/IEnumerable.cs`, `IEnumerator.cs`, `IAsyncEnumerable.cs`, `IAsyncEnumerator.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IteratorStateMachineAttribute.cs`, `AsyncIteratorStateMachineAttribute.cs`, `StateMachineAttribute.cs`, `AsyncIteratorMethodBuilder.cs`, `ConfiguredCancelableAsyncEnumerable.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Sources/ManualResetValueTaskSourceCore.cs`, `IValueTaskSource.cs`.
- `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs` (convenções de nomes `d__`, `<>2__current`).
- `docs/design/specs/runtime-async.md` (tipos de retorno permitidos).

> Nota de confiança: a forma exata do código que o Roslyn gera para `yield` (incluindo o tratamento de `try/finally` por estado) é convenção do compilador, não verificada neste repositório; o `Iterator<T>` do LINQ é o análogo escrito à mão e confirmado.
