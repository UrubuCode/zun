# Delegates vs interfaces: quando usar cada um

## Mecanismo no .NET

Ambos abstraem "algo que pode ser chamado", mas diferem no mecanismo de despacho e no que o runtime sabe sobre eles.

### Delegate

- Objeto com `_target` + `_methodPtr` (+ `_methodPtrAux`, `_extraData`, `_helperObject`) em `src/coreclr/System.Private.CoreLib/src/System/Delegate.CoreCLR.cs`.
- A invocação é uma chamada indireta ao ponteiro de método (ver arquivo 03), sem consulta à tabela virtual. O `clr-abi.md` (portable entrypoints) afirma: chamadas virtuais, de interface e de delegate "differ only in how they obtain `pep_ptr`".
- **Um único método** por tipo de delegate; a identidade do tipo é a assinatura nominal (`delegate int Cmp(T a, T b)`).
- Suporta multicast (arquivo 01), métodos estáticos e de instância, abertos e fechados, ponteiros não gerenciados.
- Igualdade: alvo + método.

### Interface

- Contrato de **zero ou mais** membros (métodos, propriedades, eventos, e membros estáticos abstratos). Implementada por tipos, resolvida por tipo dinâmico.
- Despacho de interface via **Virtual Stub Dispatch** (`docs/design/coreclr/botr/virtual-stub-dispatch.md`, `src/coreclr/vm/virtualcallstub.cpp`): lookup stub -> dispatch stub (monomórfico) -> resolve stub (polimórfico), com cache.
- Métodos de interface podem ter **implementação padrão** (`docs/design/features/default-interface-methods.md`) e **retorno covariante** (`covariant-return-methods.md`).
- Interfaces genéricas permitem **variância** (in/out) com despacho variante.
- JIT: devirtualização guardada (`docs/design/coreclr/jit/GuardedDevirtualization.md`) e inlining quando o tipo concreto é conhecido ou seladamente deduzido; **delegate invocation também pode ser otimizada** — o runtime tem otimizações de construção de delegate (`COMDelegate::GetDelegateCtor`, `DelegateCtor` em ReadyToRun: `READYTORUN_FIXUP_DelegateCtor`, `docs/design/coreclr/botr/readytorun-format.md`).

### Equivalências

| Necessidade | Delegate | Interface |
|---|---|---|
| Callback de 1 método | natural | cerimonioso (um tipo por callback) |
| Várias operações coesas | ruim | natural |
| Estado do callback | closure capturada (gera objeto) | campos da implementação |
| Multicast | nativo (`+=`) | manual |
| Estático/sem alocação | `delegate*` (arquivo 02) | `static abstract` + genéricos (constrained) |
| Variância | in/out em delegates genéricos | in/out em interfaces genéricas |
| Inlining pelo JIT | limitado (alvo dinâmico) | possível se o tipo for conhecido; genéricos por struct especializam |
| Reflection/AOT | precisa método alvo | precisa mapa de interface |

### Struct genérico com restrição de interface: o "terceiro caminho"

`where T : struct, IComparer<U>` faz o JIT **especializar por tipo de valor** e inlinar a chamada (instância de código por struct). É o padrão de zero-cost abstraction usado em bibliotecas de alto desempenho, enquanto o mesmo código com `Func<>` paga indireção por chamada. (A especialização de código genérico por tipos de valor é descrita em `docs/design/coreclr/botr/type-system.md`/`type-loader.md`; o exemplo `IComparer` é ilustrativo.)


### Mesmo problema, três soluções (C# ilustrativo)

```csharp
// (A) delegate
static int Max(int[] a, Comparison<int> cmp) { /* cmp(a[i], a[j]) */ }

// (B) interface
interface IComparer<T> { int Compare(T x, T y); }
static int Max<T>(T[] a, IComparer<T> c) { /* c.Compare(a[i], a[j]) */ }

// (C) interface + struct genérico
static int Max<T, TCmp>(T[] a, TCmp c) where TCmp : struct, IComparer<T> { /* c.Compare(...) */ }
```

- (A): chamada indireta por `_methodPtr`; o closure pode alocar.
- (B): `callvirt` de interface via VSD; o comparador é um objeto compartilhável.
- (C): como `TCmp` é tipo de valor, o runtime instancia código por struct e o JIT pode inlinar `Compare`. O repositório documenta a instanciação genérica em `type-loader.md` e `type-system.md`; a decisão de inlinar é do JIT.

### Árvore de decisão

```text
precisa de várias operações coesas ou identidade de tipo?   -> interface
senão: callback de 1 operação?
   precisa de multicast/event?                                -> delegate
   precisa capturar estado de forma ergonômica?               -> delegate (closure)
   hot path e sem estado?                                     -> delegate* ou struct genérico + interface
   precisa ser implementado por muitos tipos existentes?      -> interface
```

### O que o runtime faz diferente nos dois casos

| Aspecto | Delegate | Interface |
|---|---|---|
| Representação | objeto com `_target`/`_methodPtr` | referência ao objeto + tabela de interface |
| Resolução | na criação (binding em `comdelegate.cpp`) | na chamada (VSD) ou no JIT (devirtualização) |
| Cache | nenhum no call site (método já resolvido) | stubs por call site |
| Tipos de valor | via box ou método estático | `constrained.` + genérico evita box |
| Variância | `in`/`out` em delegates genéricos | idem em interfaces genéricas |
| Evolução binária | assinatura fixa | `default-interface-methods.md` |

### Casos concretos no próprio repositório

- Callback único: `Comparison<T>`, `Action`, `EventHandler`, delegates de `System.Private.CoreLib`.
- Contratos coesos: `IEnumerable<T>`, `IEnumerator<T>`, `IDisposable`, `IComparer<T>`.
- Generic math: `System.Numerics.IAdditionOperators` e família, projetadas para restrição genérica em vez de delegates.
- Reflexão: `CreateDelegate` converte descoberta em delegate para repetição (arquivo 08).

### Anti-padrões

- Interface de um método usada apenas como callback, com muitas classes anônimas: preferir delegate.
- Delegate multicast para obter valor de retorno agregado: semântica ruim.
- Delegates que capturam `this` em objetos de vida longa: retenção de memória.
- Interfaces enormes que forçam implementações vazias: use default methods ou separe interfaces.

### Perguntas para a sua linguagem

1. Funções são valores de primeira classe? Se sim, uma interface SAM vira desnecessária.
2. Há tipos de valor? Se sim, a combinação genérico + trait é o caminho zero-cost.
3. Qual o modelo de despacho de interface? tabelas completas, stubs, ou hash?
4. Qual o custo aceitável de uma closure (alocação, indireção)?
5. Eventos e multicast são primitivos ou bibliotecas?


### Glossário

| Termo | Significado |
|---|---|
| SAM | interface de método único, convertível em lambda em outras linguagens |
| Closure | função mais ambiente capturado |
| Monomorfização | gerar código especializado por tipo concreto |
| Megamórfico | call site com muitos tipos de receptor |

### Testes sugeridos

1. Ordenar 10 milhões de inteiros com comparador por delegate, por interface e por struct genérico.
2. Contar alocações em cada variante.
3. Verificar o inlining do comparador na variante de struct.

## Por quê

- **Delegate** nasceu para eventos e callbacks: composição leve de funções sem criar tipos.
- **Interface** nasceu para contratos de polimorfismo com múltiplos membros e identidade de tipo.
- O runtime precisa de mecanismos diferentes: delegate é uma chamada por ponteiro com `this` próprio; interface precisa de resolução por tipo, daí o stub dispatch.
- Separar também permite **otimizar cada um**: VSD para interface com cache inline; ctors de delegate especializados e shuffle thunks para estáticos.

## Trade-offs

| | Delegate | Interface |
|---|---|---|
| Alocação | closure + delegate (2 objetos às vezes) | 1 objeto de implementação (pode ser singleton/struct) |
| Custo de chamada | indireta, previsível | stub + possível miss polimórfico |
| Evolução | assinatura fixa | pode adicionar membros com default methods |
| Depuração | nome do alvo opaco (lambdas geradas) | tipos nomeados |
| Segurança AOT | simples | precisa preservar implementações |
| Composição | multicast, `Delegate.Combine` | decorator manual |
| Identidade | por método/alvo | por tipo |

- Delegate com estado capturado esconde alocações; interface exige boilerplate.
- Delegates multicast com retorno não-void têm semântica estranha.
- Interfaces com muitos métodos aumentam a tabela de despacho/itable.

## Como aplicar na sua linguagem

1. Ofereça **ambos**, mas defina a regra de ouro: 1 operação sem identidade de tipo = função/closure; ≥2 operações ou identidade = interface/trait.
2. Se tiver generics com reificação ou especialização, ofereça restrição de trait que permita **monomorfização** (struct genérico) — é o caminho de abstração sem custo.
3. Torne closures baratas: evite capturar quando possível (lambdas `static`), e considere `delegate*`-equivalente para o caso sem captura.
4. Se tiver interface com default, defina a regra de diamante (o .NET tem doc dedicado: `default-interface-methods.md`).
5. Considere permitir que uma interface de 1 método seja usada onde um closure é esperado (SAM conversion) para unificar o ecossistema.
6. Para eventos, forneça multicast como biblioteca (`Subject`/lista de closures) em vez de embutir no tipo função, a menos que queira a ergonomia `+=`.
7. Documente o custo: monomórfico (barato), polimórfico (stub/cache), megamórfico (lento).

## Referências

- `src/coreclr/System.Private.CoreLib/src/System/Delegate.CoreCLR.cs`
- `docs/design/coreclr/botr/virtual-stub-dispatch.md`, `src/coreclr/vm/virtualcallstub.cpp`
- `docs/design/coreclr/botr/clr-abi.md` (portable entrypoint / delegate calls)
- `docs/design/features/default-interface-methods.md`, `covariant-return-methods.md`
- `docs/design/coreclr/jit/GuardedDevirtualization.md`
- `docs/design/coreclr/botr/readytorun-format.md` (`READYTORUN_FIXUP_DelegateCtor`)
- `src/coreclr/vm/comdelegate.cpp`
