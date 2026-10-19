# Events e o padrão observer

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### Três camadas

1. **Delegate**: um objeto que encapsula (alvo, método) e é invocável. Tipos de delegate são classes seladas derivadas de `MulticastDelegate`, com um método `Invoke` cuja implementação é fornecida pelo runtime (C#/ECMA). `System.Delegate`: `src/libraries/System.Private.CoreLib/src/System/Delegate.cs`; `MulticastDelegate.cs` no mesmo diretório (repo).
2. **Multicast**: `Delegate.Combine(a, b)` devolve um *novo* delegate cujo invocation list é a concatenação; `Delegate.Remove(source, value)` devolve outro sem o item. No repo: `Combine` (`Delegate.cs` ~128) chama `a.CombineImpl(b)`; `CombineImpl` (~313) monta `Wrapper[]` de invocações e retorna `NewMulticastDelegate(...)`; `RemoveImpl` (~378) faz o inverso, usando `DeleteFromInvocationList`. A verificação `RuntimeHelpers.AreTypesEquivalent(this, d)` lança `ArgumentException` se os tipos de delegate diferem (repo). Delegates são *imutáveis*: toda combinação/remoção cria um novo.
3. **Event**: construção de linguagem + metadado. Em C#, `public event EventHandler Changed;` gera (C#/ECMA): um campo privado do tipo delegate; métodos `add_Changed` e `remove_Changed` (públicos, `specialname`), e um registro `Event` no metadado que liga os dois métodos. Fora do tipo, só é possível `+=` e `-=`: nunca invocar nem atribuir diretamente. Quem declara dispara com `Changed?.Invoke(this, e)`.

### Assinatura convencional

`src/libraries/System.Private.CoreLib/src/System/EventHandler.cs` (repo):

```csharp
public delegate void EventHandler(object? sender, EventArgs e);
public delegate void EventHandler<in TEventArgs>(object? sender, TEventArgs e)
    where TEventArgs : allows ref struct;
public delegate void EventHandler<in TSender, in TEventArgs>(TSender sender, TEventArgs e)
    where TSender : allows ref struct where TEventArgs : allows ref struct;
```

Observações confirmadas no arquivo: o parâmetro `TEventArgs` é contravariante (`in`); o comentário diz "Removed TEventArgs constraint post-.NET 4"; versões recentes aceitam `allows ref struct`; existe uma variação com `TSender` tipado.

### Segurança em threads

Como delegates são imutáveis e o `add_`/`remove_` gerados pelo compilador usam laço de `Interlocked.CompareExchange` sobre o campo (C#/ECMA, não verificado aqui), a assinatura/remoção concorrente é lock-free. A invocação lê o delegate atual (um snapshot): handlers adicionados durante o disparo não são chamados naquela rodada. Isso é consequência direta da imutabilidade; no repo, `CombineImpl` mostra cópia/crescimento do array de invocação em vez de mutação in-place visível (há otimização de reuso de capacidade com `TrySetSlot`/`Interlocked.CompareExchange` em `Delegate.cs` ~300-306, que escreve em slots livres do array: detalhe interno que preserva a semântica imutável).

### Dispatch do Invoke

O `Invoke` de um delegate não passa pela vtable normal: o runtime fornece o corpo (`MethodDesc` de categoria EEImpl, ver `docs/design/coreclr/botr/method-descriptor.md`: "EEImpl: Delegate methods whose implementation is provided by the runtime (Invoke, BeginInvoke, EndInvoke, ...)") (repo: o doc diz "Delegate methods whose implementation is provided by the runtime (Invoke, BeginInvoke, EndInvoke)"). Para um delegate com um único destino, é um salto indireto via campos do objeto (alvo + ponteiro de código); para multicast, um laço sobre o invocation list (inferência baseada em `Delegate.cs`).

### Eventos e o GC

O vínculo `fonte -> handler` cria uma referência forte da fonte para o observador (o delegate guarda `target`). Esquecer de remover o handler vaza o observador enquanto a fonte viver (C#/ECMA, problema clássico "lapsed listener"). O repo tem tipos de referência fraca (`WeakReference`, `DependentHandle`, etc.) que podem ser usados em padrões "weak event" (não detalhado aqui).

### Eventos de runtime/telemetria

"Event" também é usado para tracing (`EventSource`, EventPipe). É outro conceito (publicação para um listener fora do processo/no processo); não confundir com o padrão de linguagem. Ex.: `src/libraries/System.Private.CoreLib/src/System/Diagnostics/Tracing/EventSource.cs` (repo).

## Por quê

1. **Observer embutido na linguagem.** Delegates multicast + `event` dão o padrão sem classes `Observer`/`Subject` manuais; o handler é qualquer método com a assinatura certa.
2. **Encapsulamento do disparo.** `event` restringe o acesso externo a `+=`/`-=`: só o dono invoca. Um campo delegate público permitiria a qualquer um invocar ou limpar a lista (C#/ECMA).
3. **Tipagem de callback.** Delegates são verificados estaticamente (contra/covariância).
4. **Imutabilidade para thread-safety barata.** Registro e remoção podem ser lock-free (C#/ECMA).
5. **Convenção uniforme** (`sender`, `EventArgs`) para ferramentas e designers (UI, binding).

## Trade-offs

- **Vazamento por referência forte** (lapsed listener).
- **Ordem e exceções**: uma exceção em um handler interrompe os seguintes na invocação multicast padrão (C#/ECMA); retorno de múltiplos handlers perde todos menos o último.
- **Custo de alocação**: cada `+=` aloca novo delegate e novo array (O(n) por registro; O(n²) para n registros sequenciais). Em `CombineImpl` do repo há otimização que arredonda a capacidade para potência de 2 (`BitOperations.RoundUpToPowerOf2`) e reutiliza espaço para amortizar.
- **Sem garantia de ordem formal** além da ordem de inscrição.
- **Disparo durante modificação**: snapshot semântico; handlers removidos ainda podem ser chamados uma vez após `-=` por corrida (C#/ECMA).
- **Eventos não são valores de primeira classe**: não dá para passar "o evento" como argumento; só `add`/`remove` indiretos (C#/ECMA). Alternativas modernas: `IObservable<T>`, canais, `async` streams.
- **Dois níveis de conceito** (delegate vs event) confundem iniciantes.

## Como aplicar na sua linguagem

1. **Base mínima**: funções/closures como valores de primeira classe (delegate). O resto é biblioteca.
2. **Multicast como valor imutável** (lista persistente) com combine/remove que retornam novos valores. Copy-on-write dá thread-safety simples; use CAS no campo.
3. **Event como açúcar de acesso restrito**: declare `event E: Handler` gerando campo privado + `add_E`/`remove_E` públicos e uma forma de disparo só para o dono. Em uma linguagem com propriedades, reaproveite o conceito: um event é uma "propriedade" de duas operações (add/remove).
4. **Defina a semântica de falhas**: parar no primeiro erro, juntar erros (AggregateException), ou continuar. O .NET para; considere oferecer `InvokeAll` que coleta.
5. **Defina o retorno de multicast**: restrinja eventos a `void` ou ofereça agregador explícito.
6. **Considere referências fracas** como opção de primeira classe (`+=weak`) para evitar o lapsed listener, ou então tenha um tipo `Subscription` com `dispose` (padrão do `IObservable`).
7. **Variância**: o `EventHandler<in TEventArgs>` do repo mostra que contravariância no argumento é útil (handler que aceita `EventArgs` pode tratar `MyArgs`).
8. **Thread/contexto**: decida em que thread/contexto handlers rodam (síncrono no chamador, como no .NET; ou fila). Documentar.
9. **Async**: se a linguagem tem `async`, decida se handlers podem ser assíncronos e como o disparo os aguarda (o `async void` handler do .NET é fonte de bugs).

Lowering de referência:

```
event Changed: Handler
=>  field  _Changed: Handler?
    fn add_Changed(h)    { loop { old=_Changed; new=Combine(old,h);   if CAS(&_Changed, old, new) break } }
    fn remove_Changed(h) { loop { old=_Changed; new=Remove(old,h);    if CAS(&_Changed, old, new) break } }
    // disparo (só dentro do tipo): let d=_Changed; if d != null { d(sender, args) }
```

## Resumo dos fatos confirmados

- `EventHandler`, `EventHandler<TEventArgs>` (contravariante) e `EventHandler<TSender, TEventArgs>` estão em `EventHandler.cs`, com `allows ref struct`.
- `Delegate.Combine` -> `CombineImpl` produz um novo `MulticastDelegate`; `Remove` -> `RemoveImpl` idem.
- `CombineImpl` exige tipos de delegate equivalentes (`AreTypesEquivalent`) e lança `ArgumentException` caso contrário.
- Capacidade do array de invocação cresce em potências de 2 (`BitOperations.RoundUpToPowerOf2`).
- Existe otimização que reaproveita folga no array (`TrySetSlot`) para evitar cópia.
- Métodos do delegate são `EEImpl` (fornecidos pelo runtime).

## Fluxo de um evento (modelo)

1. Observador cria delegate para seu método de tratamento.
2. `fonte.Evento += handler` chama `add_Evento`.
3. `add_Evento` combina o delegate atual com o novo, de forma atômica (padrão com CAS, C#/ECMA).
4. Fonte dispara: lê o campo uma vez, invoca; cada handler roda em ordem, na mesma thread.
5. `fonte.Evento -= handler` chama `remove_Evento`.
6. Se o observador não se desinscreve, a fonte mantém o observador vivo (referência forte).

## Padrões relacionados

- `IObservable<T>`/`IObserver<T>`: observer baseado em interfaces e `IDisposable` como assinatura.
- Eventos fracos: usar referências fracas no registro (não faz parte do delegate padrão).
- `INotifyPropertyChanged`: padrão de evento sobre propriedades, usado em binding.
- Async/await: evita encadear callbacks em muitos cenários, mas não substitui pub/sub.
- Canais/streams: modelo alternativo para múltiplos produtores/consumidores.

## Perguntas de design

- A ordem de notificação é garantida? (no .NET é a ordem de inscrição)
- Exceção em um handler interrompe os outros?
- Retornos de handlers: ignorados, agregados, ou proibidos?
- Reentrância: um handler pode se inscrever/desinscrever durante o disparo?
- Threading: handlers executam no thread do disparo, ou em um contexto/despachante?
- Quem é responsável por desinscrever? Há suporte a tempo de vida (escopo, `Disposable`, referência fraca)?
- O evento é um valor de primeira classe (pode ser passado como argumento)?

## Glossário

- Delegate: objeto que encapsula alvo e método e é invocável.
- Multicast: delegate com lista de invocação de vários destinos.
- Invocation list: lista interna de destinos de um delegate multicast.
- Event: construção que expõe apenas inscrição e remoção ao exterior.
- Handler: método registrado num evento.
- `sender`: parâmetro convencional que identifica a fonte do evento.
- `EventArgs`: classe base convencional para dados de evento.
- Lapsed listener: observador que permanece vivo por não ter se desinscrito.
- Snapshot de invocação: o delegate lido no momento do disparo, imutável.

## Protótipo mínimo (pseudocódigo, proposta minha)

```
type Handler<A> = fn(sender: any, args: A)

class Evento<A> {
    private lista: [Handler<A>] = []      // imutável: cada mudança cria nova lista
    fn add(h)    { atomic { lista = lista + [h] } }
    fn remove(h) { atomic { lista = lista sem a última ocorrência de h } }
    internal fn dispara(sender, args) {
        let snapshot = lista
        for h in snapshot { h(sender, args) }
    }
}
```

Notas:

- A cópia na mudança espelha a imutabilidade do `MulticastDelegate`.
- `remove` retira a última ocorrência no .NET (comportamento de `Delegate.Remove`, C#/ECMA).
- O snapshot evita problemas de modificação durante a iteração.
- Para exceções, escolha entre parar no primeiro erro (como o .NET) e coletar.
- Para evitar vazamento, adicione `Subscription` com `dispose()` que chama `remove`.
- Para vínculos fracos, guarde `WeakRef<alvo>` e o método separadamente.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/EventHandler.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Delegate.cs`: `Combine` (~128), `Remove` (~172), `CombineImpl` (~313), `RemoveImpl` (~378), `DeleteFromInvocationList`.
- `src/libraries/System.Private.CoreLib/src/System/MulticastDelegate.cs`.
- `docs/design/coreclr/botr/method-descriptor.md`: categoria EEImpl de MethodDesc.
- `src/libraries/System.Private.CoreLib/src/System/Diagnostics/Tracing/EventSource.cs` (conceito distinto de "evento").
