# Delegates: multicast e lista de invocação

## Mecanismo no .NET

Um delegate é um objeto gerenciado que encapsula "um alvo + um ponteiro de código" e é invocável por um método sintético `Invoke`. Na versão deste repositório, os campos de estado vivem em um único lugar:
`src/coreclr/System.Private.CoreLib/src/System/Delegate.CoreCLR.cs`.

### Layout do objeto (CoreCLR)

| Campo | Papel (conforme comentários no código) |
|---|---|
| `_target` | objeto sobre o qual invocar (o "closure"/`this`) |
| `_methodPtr` | ponteiro para o código a invocar; pode ser um *thunk* pequeno se o método for estático ou UM |
| `_methodPtrAux` | em delegate de método estático, guarda o que `_methodPtr` guardaria; `_methodPtr` aponta então para um thunk que descarta o `this` e salta para `_methodPtrAux` |
| `_extraData` | contagem multicast, o marcador `UnmanagedMarker = -1`, ou o `MethodDesc` do alvo |
| `_helperObject` | usado em 3 casos: multicast (`Wrapper[]`), cache de método (`MethodInfo`), delegates colecionáveis (`LoaderAllocator`) |

Regras derivadas do próprio código:

- `IsClosed => _methodPtrAux == 0` (delegate "fechado": `_target` é o primeiro argumento / `this`).
- `IsUnmanagedFunctionPtr => _extraData == UnmanagedMarker` (delegate criado a partir de ponteiro de função não gerenciado).
- O comentário diz explicitamente que `_target` e `_methodPtr` ficam adjacentes "para desempenho ótimo de invocação de delegate".

Observação importante: em versões antigas do runtime o multicast tinha `_invocationList`/`_invocationCount` em `MulticastDelegate`. **Neste checkout** a lógica multicast foi movida para `Delegate` (`src/libraries/System.Private.CoreLib/src/System/Delegate.cs`), e `MulticastDelegate.cs` (60 linhas) só contém construtores e `operator ==`/`!=`. Não presuma a forma antiga.

### Lista de invocação multicast

- `Delegate.Combine(a, b)` retorna `b` se `a` é nulo; senão chama `a.CombineImpl(b)` (`Delegate.cs`, ~linha 128 e 313).
- `CombineImpl` constrói um novo array `Wrapper[]` com a concatenação das listas (`resultCount = invocationList.Length + followList.Length`) e chama `NewMulticastDelegate(Wrapper[] invocationList, int invocationCount, bool thisIsMultiCastAlready)` (`Delegate.CoreCLR.cs`, ~linha 623), que seta `_helperObject = invocationList` e `_extraData = invocationCount`.
- `TryGetInvocations(out ReadOnlySpan<Wrapper>)` devolve `false` para delegate de alvo único (`HasSingleTarget`); caso contrário fatia o array até `_extraData`. Os asserts documentam invariantes: `invocationList.Length > 1`, `Length >= _extraData`, `invocationList[0].Value != null`. Ou seja, o array pode ter **capacidade maior que a contagem**.
- `Remove`/`RemoveImpl` usa `LastIndexOf` (remove a **última** ocorrência da sub-lista). Se sobram 2 elementos e um é removido, devolve o outro delegate diretamente (volta a ser alvo único). Caso contrário `DeleteFromInvocationList` aloca array com tamanho arredondado para potência de 2 (`BitOperations.RoundUpToPowerOf2`).
- `GetInvocationList()` copia os `Wrapper` para um `Delegate[]`. Existe também `Delegate.EnumerateInvocationList<TDelegate>(d)` que devolve um `InvocationListEnumerator<TDelegate>` (struct, padrão duck-typed de `foreach`) para iterar **sem alocar** o array.
- `Delegate.Target` num multicast retorna o `Target` do **último** item (`invocations[^1]`).
- `MulticastDelegate.operator ==` testa `d2` primeiro para permitir eliminação de ramos quando inlinado em `== null` (comentário no código).

Semântica de resultado (ECMA-335/C#, conhecimento geral, não verificado aqui): invocar um multicast chama cada item em ordem; o valor de retorno é o do último; uma exceção interrompe a cadeia.

### Imutabilidade

`Combine`/`Remove` nunca alteram o delegate original: sempre devolvem outro objeto (ou o próprio argumento). Isso torna `event += handler` com `Interlocked.CompareExchange` seguro sem locks — padrão copy-on-write confirmado pela estrutura do código (novo `Wrapper[]` a cada combinação).

### Criação e binding no VM

`src/coreclr/vm/comdelegate.cpp` (2739 linhas) contém:

- `COMDelegate::BindToMethod` — resolve o método alvo.
- `COMDelegate::GetDelegateCtor` — escolhe o construtor/otimização conforme o tipo do alvo.
- `COMDelegate::GetInvokeMethodStub` — stub para `Invoke`.
- `COMDelegate::GetMethodDescForOpenVirtualDelegate` — delegates abertos sobre método virtual.
- `ValidateDelegatePInvoke`, `GetILStubMethodDesc` — delegates usados em interop (ver arquivo 04).
- Código de "shuffle thunk" (`GenerateShuffleArray`, `FEATURE_PORTABLE_SHUFFLE_THUNKS`): quando um delegate estático é chamado, os argumentos precisam ser deslocados um slot para ocupar o lugar do `this` ausente.

`docs/design/coreclr/botr/method-descriptor.md` registra que `Invoke`/`BeginInvoke`/`EndInvoke` são "Delegate methods whose implementation is provided by the runtime" e que existe `ThisPtrRetBufPrecode` para trocar return-buffer e `this` em delegates abertos de instância que retornam value types.


### Os quatro "sabores" de delegate (como os campos se combinam)

A partir dos comentários de `Delegate.CoreCLR.cs`, os estados válidos podem ser resumidos assim:

| Sabor | `_target` | `_methodPtr` | `_methodPtrAux` | Observação |
|---|---|---|---|---|
| Fechado sobre instância | o objeto | código do método | 0 (`IsClosed`) | caso mais rápido |
| Estático | definido pelo construtor no VM | thunk que descarta o `this` | código real do método | custo extra do shuffle |
| Aberto de instância | não usado como `this` | código ou stub virtual | não nulo | `this` vem como 1o argumento do `Invoke` |
| Ponteiro não gerenciado | n/a | thunk de reverse P/Invoke | n/a | `_extraData == -1` |

Nota de honestidade: o valor exato de `_target` no caso estático depende do código do construtor em `comdelegate.cpp`; o que está confirmado no `.cs` é o comentário sobre `_methodPtrAux` e a definição de `IsClosed`.

### Fluxo de `d1 + d2` passo a passo

```text
Delegate.Combine(a, b)
  a == null            -> retorna b
  senão                -> a.CombineImpl(b)
CombineImpl(d)
  TryGetInvocations(this) -> lista de 'this' (ou 1 item se alvo único)
  d com lista própria     -> lista 'followList' (ou 1 item)
  resultCount = len(this) + len(follow)
  aloca Wrapper[] (capacidade >= resultCount) e copia as duas listas
  NewMulticastDelegate(array, resultCount, thisIsMultiCastAlready)
     -> cria delegate do mesmo tipo, com _helperObject=array, _extraData=count
```

Pontos confirmados em `Delegate.cs` e `Delegate.CoreCLR.cs`: o `Wrapper` embrulha o `Delegate` de cada item (`invocations[i].Value`); `RemoveImpl` compara com `new Wrapper(d)`; `DeleteFromInvocationList` copia dois trechos do array antigo para um novo array.

### Padrão canônico de evento thread-safe (uso da API, conhecimento geral)

```csharp
EventHandler? h = _evt;
EventHandler? n;
do {
    n = (EventHandler?)Delegate.Combine(h, value);
} while ((h = Interlocked.CompareExchange(ref _evt, n, h)) != h);
```

Isto só é correto porque `Combine` é puro (devolve objeto novo). Se sua linguagem tornar a lista mutável, precisará de lock.

### Perguntas de projeto que o código do .NET responde

- A lista deve ser compartilhada entre delegates derivados? `Remove` só cria array novo quando necessário; o caso de 2 itens devolve o item restante sem criar multicast.
- Quem é o `Target` de um multicast? O do último item (`invocations[^1]`).
- Como iterar sem alocar? `Delegate.EnumerateInvocationList<TDelegate>` com struct enumerador.
- Delegates de assemblies descarregáveis? `_helperObject` guarda o `LoaderAllocator` (3o uso do campo, segundo o comentário).


### Testes sugeridos para a sua implementação

1. Combinar A+B+C e remover B: a ordem restante deve ser A, C.
2. Combinar o mesmo delegate duas vezes e remover uma vez: sobra uma ocorrência (remoção da última, como `LastIndexOf`).
3. Remover até sobrar 1: o resultado deve voltar ao caso de alvo único.
4. Invocar multicast cujo item do meio lança: definir se os seguintes executam.
5. Delegate sobre método de instância com `this == null` no momento da criação: definir se erra na criação ou na chamada.
6. Igualdade de dois delegates para o mesmo método/alvo: devem ser iguais.

### Glossário

| Termo | Significado |
|---|---|
| Fechado | `_methodPtrAux == 0`; o alvo é o `this` |
| Aberto | o `this` é fornecido na chamada |
| Wrapper | struct que embrulha cada `Delegate` da lista |
| Shuffle thunk | código que reordena argumentos para adaptar convenções |

## Por quê

- **Uma abstração uniforme de "callable"**: estático, instância fechada (captura `this`), instância aberta (this vira 1º parâmetro) e função não gerenciada compartilham a mesma forma de invocação.
- **Invocação rápida**: `Invoke` vira essencialmente `call [_methodPtr]` com `_target` como `this`; o custo do caso comum (alvo único) não paga pelo multicast.
- **Eventos**: lista imutável + `Combine`/`Remove` dão o modelo de publish/subscribe sem estrutura de coleção própria.
- **Segurança de tipos**: o assinatura do delegate é verificada na criação (binding), não na chamada.

## Trade-offs

- Estático via thunk de shuffle é mais lento que instância fechada (o runtime documenta o thunk exatamente por isso).
- Multicast imutável é O(n) por `+=`/`-=` e gera lixo; muitos assinantes viram O(n²) total.
- Valor de retorno de multicast ambíguo (só o último), e exceção aborta os demais — armadilha conhecida; `GetInvocationList` é necessário para controle fino.
- Delegates capturam `_target` com forte referência: vazamentos de memória por eventos são clássicos (inferência geral).
- Igualdade de delegate compara alvo + método (ver `Equals` em `Delegate.CoreCLR.cs`: compara `_target` e `_methodPtr`), o que surpreende com lambdas.
- Delegates colecionáveis exigem `_helperObject` extra para manter o `LoaderAllocator` vivo.

## Como aplicar na sua linguagem

1. Defina o valor função como par `(env, code)`; mantenha os dois campos adjacentes e o caso "alvo único" sem indireção extra.
2. Decida cedo se multicast é primitivo da linguagem. Alternativa: multicast como biblioteca (lista imutável de callables) e o tipo base permanece simples.
3. Se escolher o modelo .NET: array com capacidade ≥ contagem, `Combine` copy-on-write, `Remove` pela última ocorrência, e um enumerador sem alocação desde o dia 1.
4. Especifique a semântica de retorno e de exceção do multicast na linguagem (o .NET deixa a cargo do chamador usar a lista).
5. Separe "método estático" de "closure" no formato, ou use um thunk único que descarta o `this` — o .NET escolheu o thunk de shuffle para manter uma só convenção de chamada de `Invoke`.
6. Considere um marcador para "ponteiro não gerenciado" no mesmo tipo (`UnmanagedMarker = -1`), unificando interop e callbacks.
7. Planeje a relação com GC/descarregamento: um campo auxiliar para manter o dono do código vivo.

## Referências

- `src/coreclr/System.Private.CoreLib/src/System/Delegate.CoreCLR.cs` (campos, `TryGetInvocations`, `NewMulticastDelegate`)
- `src/libraries/System.Private.CoreLib/src/System/Delegate.cs` (`Combine`, `CombineImpl`, `RemoveImpl`, `GetInvocationList`, `EnumerateInvocationList`)
- `src/libraries/System.Private.CoreLib/src/System/MulticastDelegate.cs`
- `src/coreclr/nativeaot/Runtime.Base/src/System/MulticastDelegate.cs` (variante NativeAOT, implementação distinta)
- `src/coreclr/vm/comdelegate.cpp`, `src/coreclr/vm/comdelegate.h`
- `docs/design/coreclr/botr/method-descriptor.md`
- `docs/design/coreclr/botr/clr-abi.md` (delegate calls na seção de ABI portátil)
