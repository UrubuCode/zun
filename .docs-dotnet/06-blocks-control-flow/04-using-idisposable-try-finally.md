# using / IDisposable como açúcar de try/finally

No .NET, `using` não existe no IL nem no runtime. É uma **transformação do compilador** sobre duas coisas que o runtime oferece: a interface `IDisposable` (um contrato por convenção) e a região `try/finally` (um mecanismo de IL). Este documento separa claramente o que o runtime fornece do que é convenção de linguagem.

## Mecanismo no .NET

### 1. O contrato: `IDisposable` e `IAsyncDisposable`

`src/libraries/System.Private.CoreLib/src/System/IDisposable.cs` declara uma interface com um único método `Dispose()`. O comentário do arquivo é uma fonte primária valiosa e diz textualmente:

> "C# has added a using clause, which will generate a try/finally statement where the resource passed into the using clause will always have its Dispose method called. Syntax is using(FileStream fs = ...) { .. };"

E enumera as condições esperadas de um `Dispose` correto:

1. ser seguro chamar várias vezes (idempotente);
2. liberar os recursos da instância;
3. chamar o `Dispose` da classe base, se necessário;
4. suprimir a finalização (`GC.SuppressFinalize`) para aliviar a fila de finalização;
5. não lançar exceções, exceto erros graves e inesperados (como `OutOfMemoryException`).

O mesmo comentário explica a motivação: o GC não permite saber deterministicamente quando um finalizador rodará, então classes que seguram recursos do SO ou estado importante (ex.: `FileStream`) precisam de uma forma determinística de limpeza. Também registra uma alternativa que foi **considerada e descartada**: usar a interface como marcador para o compilador exigir limpeza em todos os caminhos ("any compiler that draconian may tick off any number of people").

A versão assíncrona, `IAsyncDisposable` (`src/libraries/System.Private.CoreLib/src/System/IAsyncDisposable.cs`), tem `ValueTask DisposeAsync()`. Suporte auxiliar: `ConfiguredAsyncDisposable` e `ConfiguredCancelableAsyncEnumerable` (em `Runtime/CompilerServices/`) encaminham `DisposeAsync()` com `ConfigureAwait`.

### 2. A expansão (convenção do compilador C#)

O compilador C# não está neste repositório, então a forma exata do IL gerado não é verificável aqui. A semântica **documentada pelo comentário acima** é "try/finally que sempre chama Dispose". A forma canônica conhecida (fora deste repo, tratar como convenção) é:

```csharp
using (var r = Acquire()) { Body(r); }
// equivale a
var r = Acquire();
try { Body(r); }
finally { if (r != null) r.Dispose(); }
```

Pontos de design a notar (convenção, não contrato do runtime):
- o recurso é avaliado **antes** do `try`; se `Acquire()` lançar, não há `Dispose`;
- o teste de `null` existe porque `using` aceita expressões que podem ser nulas;
- para tipos valor, o compilador evita boxing chamando `Dispose` diretamente (via `constrained.` callvirt); o IL dispõe do prefixo `constrained.` (`src/coreclr/inc/opcode.def`, linha 316) para chamadas virtuais sobre tipos valor sem boxing; que o `using` o utilize é convenção do compilador.

### 3. O que o runtime faz com esse `try/finally`

Tudo o que foi descrito nos docs 2 e 3 se aplica:

- O `finally` é uma cláusula `COR_ILEXCEPTION_CLAUSE_FINALLY` (`src/coreclr/inc/corhdr.h`, linha 1153).
- Saída normal: o corpo termina com `leave`, e o JIT, em `Compiler::impImportLeave` (`src/coreclr/jit/importer.cpp`, linha 4743), cria `BBJ_CALLFINALLY` + `BBJ_CALLFINALLYRET`. Em muitos casos o finally é **clonado inline** (`docs/design/coreclr/jit/finally-optimizations.md`, `clr-abi.md` "Cloned finallys"), de modo que o caminho feliz de um `using` tem custo próximo de chamar `Dispose` diretamente.
- Saída excepcional: o `finally` roda na **segunda passada** (`InvokeSecondPass`, `src/coreclr/nativeaot/Runtime.Base/src/System/Runtime/ExceptionHandling.cs`, linha ~1188).
- **Exceção não tratada**: se nenhum handler for encontrado na primeira passada, o runtime chama o handler de exceções não tratadas e falha o processo **sem executar a segunda passada** (assert da linha ~877). Logo, `using` não garante `Dispose` num crash por exceção não tratada.

### 4. `using` com `Lock.Scope`: struct descartável sem alocação

`System.Threading.Lock` (`src/libraries/System.Private.CoreLib/src/System/Threading/Lock.cs`, linhas 155-190) expõe `EnterScope()` retornando `Lock.Scope`, um `ref struct` com `Dispose()` que sai do lock. É um exemplo de padrão idiomático: **`using var scope = lock.EnterScope();`** vira `try/finally` sobre um tipo valor, sem alocação em heap. Detalhes no doc 7.

Outros `ref struct` descartáveis: o padrão "pattern-based using" (um `ref struct` não pode implementar interface, então o compilador aceita qualquer tipo com método `Dispose()` acessível). Isso é visível no repositório pelo fato de `Lock.Scope` ser `ref struct` com método `Dispose()` público e sem implementar `IDisposable` (ver declaração em `Lock.cs`: `public ref struct Scope`).

### 5. `await using` / `IAsyncDisposable`

Mesma ideia, com `await` dentro do `finally`. Isso interage com o fato de o runtime-async **proibir suspensão dentro de handlers** (`docs/design/specs/runtime-async.md`: "Suspension points may not appear in a handler block (`catch`, `filter`, `finally`, or `fault`)"). Portanto, `await using` não pode ser compilado como um simples `await` dentro de `finally` na implementação runtime-async; o compilador precisa reestruturar (ex.: capturar a exceção, sair do handler, aguardar o `DisposeAsync`, relançar). Essa reestruturação é convenção do compilador e não é detalhada neste repositório. Já com a máquina de estados do compilador, o finally é reescrito pelo próprio compilador (doc 5).

### 6. Finalizadores como rede de segurança

O comentário de `IDisposable.cs` recomenda que classes com recursos definam um finalizador que chame `Dispose`, quando possível, e dá o exemplo `Stream`/`StreamWriter` para mostrar por que nem sempre é viável (a ordem de finalização não é determinística entre objetos). `Stream.cs` usa `GC.SuppressFinalize(this)` em seu padrão `Dispose()` (linha ~164). Resumo: `using` é o caminho primário; finalizador é fallback.

### 7. Preservação de exceção no `Dispose`

Se o corpo do `using` lança e o `Dispose` também lança, a exceção do `Dispose` **substitui** a original (comportamento natural de uma exceção lançada dentro de `finally` na segunda passada: uma nova exceção inicia uma nova busca; ver o conceito de "collided unwind" no doc 3). Por isso o contrato pede que `Dispose` não lance.

## Por quê

1. **Determinismo sem custo de runtime especial.** O runtime não precisa de "escopos com destrutores" ou contagem de referências: o GC cuida da memória e `try/finally` cuida de recursos.
2. **Reuso de um mecanismo existente.** Em vez de criar uma instrução nova, o design reutiliza `try/finally`, que o JIT já otimiza (clonagem de finally) e o runtime já despacha (2ª passada).
3. **Contrato mínimo.** Uma interface com um método. Compiladores de outras linguagens (VB `Using`, F# `use`, C++/CLI) podem implementar a mesma transformação.
4. **Evitar um compilador "draconiano".** O comentário do fonte explicitamente rejeita checagem obrigatória de descarte por compilador; a escolha foi dar sintaxe conveniente, não exigir.

## Trade-offs

| Escolha | Vantagem | Custo |
|---|---|---|
| Descarte opt-in via convenção | Simplicidade, interoperabilidade entre linguagens | Esquecer `using` vaza recursos até o finalizador (se existir) |
| `Dispose` idempotente por contrato | Seguro em múltiplos caminhos | Todo implementador precisa de flag de estado (`_disposed`) |
| Reuso de `try/finally` | Zero novas instruções; JIT otimiza | Não roda em exceção não tratada; erro no Dispose mascara a exceção original |
| `Dispose` não deve lançar | Previsível | Não é imposto; violação silenciosa |
| `ref struct` com `Dispose` (pattern-based) | Sem alocação, sem interface | Funcionalidade depende de regra específica do compilador |
| `IAsyncDisposable` | Libera recursos assíncronos | Complica com await em finally (restrição runtime-async, ver doc 5) |
| Finalizador como rede de segurança | Limpeza eventual | Não determinístico; custo na fila de finalização; ordem indefinida |

## Como aplicar na sua linguagem

1. **Ofereça uma construção de escopo de recurso** (`using`, `defer`, `with`) definida como açúcar sobre sua região protegida de limpeza. Use `fault`/`finally` da sua IR; não crie um mecanismo separado.
2. **Defina o contrato de descarte numa interface/trait mínima** (um método) e documente: idempotente, não lança, libera recursos.
3. **Especifique a ordem**: variáveis `using` são descartadas em ordem inversa à declaração (decorre de aninhar try/finally; defina explicitamente).
4. **Decida sobre exceções do Dispose**: o .NET deixa a nova exceção substituir a antiga. Alternativas: agregar (`AggregateException`) ou suprimir e anexar à primária. Documente o que escolher.
5. **Suporte tipos valor sem alocação** (como `Lock.Scope`): permita descarte por *padrão* (método com nome conhecido) e não só por interface, evitando boxing.
6. **Planeje o caso assíncrono desde o início.** Se seu runtime proíbe suspensão em handlers (como o runtime-async do .NET), a construção `async using` precisa de uma tradução própria. Decida se o handler pode suspender ou se o compilador reescreve.
7. **Combine com move/ownership se sua linguagem tiver**: ownership estático (tipo Rust) torna o descarte garantido pelo compilador e dispensa `finally` na maioria dos casos; o modelo .NET é o de "convenção + sintaxe".
8. **Decida o comportamento em término anormal**: o .NET não executa `finally` em exceção não tratada (falha rápida); sua linguagem pode escolher outra política, mas deve documentar.
9. **Considere análise estática**: o .NET deliberadamente não exige; ferramentas externas (analisadores) fazem esse papel. Uma linguagem nova pode embutir o aviso "recurso criado e não descartado".

## Apêndice: caminhos de execução de um `using`

Tabela baseada nos mecanismos já confirmados (docs 2 e 3). A coluna "mecanismo" aponta o código que implementa cada passo.

| Caminho | O que acontece | Mecanismo |
|---|---|---|
| Corpo termina normalmente | `leave` para depois da região; o finally roda (inline se clonado) | `impImportLeave` (`importer.cpp` ~4743); finally clonado (`clr-abi.md`) |
| `return x;` dentro do corpo | O valor é guardado em local; `leave` até o único `ret` fora das regiões | `ret` não pode estar dentro de região protegida (citação da ECMA-335 em `flowgraph.cpp` ~1500) |
| `break`/`continue`/`goto` para fora | `leave` para o rótulo; o finally executa antes do salto final | blocos "step" criados por `impImportLeave` |
| Exceção capturada em frame externo | 1ª passada acha o catch; 2ª passada chama o finally do `using` e depois o catch | `DispatchEx` / `InvokeSecondPass` (`ExceptionHandling.cs`) |
| Exceção filtrada (`when`) externa | O filtro roda **antes** do `Dispose` | filtros rodam na 1ª passada (doc 3) |
| Exceção não tratada | Nenhum `Dispose` via finally; o processo termina | `UnhandledExceptionFailFastViaClasslib` (`ExceptionHandling.cs` ~877) |
| `Dispose` lança durante unwinding | Nova exceção substitui a anterior e a busca recomeça da cláusula seguinte | "collided unwind" (`startIdx`, `FindFirstPassHandler`) |
| Aquisição lança | O `try` ainda não começou, portanto não há `Dispose` | semântica da expansão (recurso avaliado antes do `try`) |

## Apêndice: perguntas de design que o `using` do .NET responde (e que você deve responder)

1. Quem é responsável pela limpeza: o chamador (`using`) ou o objeto (finalizador)? O .NET escolhe ambos, com precedência ao chamador.
2. A limpeza é parte do sistema de tipos (interface) ou da sintaxe? No .NET é parte do sistema de tipos (`IDisposable`) mas só consumida por sintaxe; `Lock.Scope` mostra que também pode ser por padrão estrutural.
3. A limpeza pode falhar? O contrato desencoraja, mas o runtime não impede.
4. A limpeza roda em todos os términos? Não: não roda em exceção não tratada (confirmado no despachante, doc 3).
5. A limpeza pode ser assíncrona? Sim (`IAsyncDisposable`), com a ressalva de suspensão em handlers do runtime-async.
6. Pode-se reusar o objeto após `Dispose`? Convenção: não; os tipos lançam `ObjectDisposedException` (`ObjectDisposedException.cs` na BCL).
7. Quem chama `GC.SuppressFinalize`? O próprio `Dispose` (item 4 das condições no comentário de `IDisposable.cs`).
8. O que acontece com tipos valor? Pode-se evitar boxing via `constrained.`; descarte duplo de cópias de struct é fonte clássica de bugs, por isso `ref struct` como `Lock.Scope` zera seu campo ao descartar (idempotência explícita).

## Apêndice: padrão de `Dispose` recomendado pelo comentário da BCL

A lista de condições de `IDisposable.cs` pode ser lida como um modelo de implementação (esboço, não código do repositório):

```csharp
class Recurso : IDisposable
{
    private bool _disposed;
    public void Dispose()
    {
        if (_disposed) return;          // 1) seguro chamar várias vezes
        _disposed = true;
        LiberaRecursos();               // 2) libera recursos da instância
        base_Dispose();                 // 3) chama o Dispose da base, se houver
        GC.SuppressFinalize(this);      // 4) evita a fila de finalização
    }                                   // 5) não lança (idealmente)
}
```

Nota: o repositório também cita o exemplo de `Stream` (`Stream.cs`, linha ~164: `GC.SuppressFinalize(this)`) como implementação real desse padrão.

## Apêndice: `using` e o resto do capítulo

- `using` → região `finally` (doc 2) → clonagem do finally e `leave` (doc 9).
- `lock` com `Lock.EnterScope()` → `using` sobre `ref struct` (doc 7).
- `await using` → conflito com a regra de runtime-async de não suspender em handlers (doc 5).
- `foreach` sobre `IEnumerator<T>` → `using` implícito para chamar `Dispose` do enumerador, que dispara os `finally` pendentes de iteradores (doc 6; o descarte pelo `foreach` é convenção do compilador).
- Closures não capturam variáveis `using` sem cuidado: o recurso pode ser descartado antes da execução do delegate (doc 12). Esta é uma consequência lógica de o `Dispose` ocorrer no fim do bloco, enquanto a closure estende a vida do objeto.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/IDisposable.cs` (comentário de design e condições do `Dispose`).
- `src/libraries/System.Private.CoreLib/src/System/IAsyncDisposable.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Threading/Lock.cs` (`EnterScope`, `Lock.Scope`).
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ConfiguredAsyncDisposable.cs`, `ConfiguredCancelableAsyncEnumerable.cs`.
- `src/libraries/System.Private.CoreLib/src/System/IO/Stream.cs` (uso de `GC.SuppressFinalize`).
- `src/coreclr/inc/corhdr.h` (cláusula `FINALLY`), `src/coreclr/jit/importer.cpp` (`impImportLeave`).
- `docs/design/coreclr/jit/finally-optimizations.md`, `docs/design/coreclr/botr/clr-abi.md` (cloned finallys).
- `docs/design/specs/runtime-async.md` (suspensão proibida em handlers).
- `src/coreclr/nativeaot/Runtime.Base/src/System/Runtime/ExceptionHandling.cs` (`InvokeSecondPass`, falha em exceção não tratada).

> Nota de confiança: a expansão exata do IL do `using` é convenção do compilador C# (fora deste repositório); o comentário em `IDisposable.cs` é a evidência local da equivalência com `try/finally`.
