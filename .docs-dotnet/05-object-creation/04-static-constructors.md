# Static constructors (.cctor) e flag beforefieldinit

## Mecanismo no .NET

### O que é

Um *type initializer* (`.cctor`, "class constructor") é um método estático especial sem
parâmetros que inicializa o estado estático do tipo: campos `static` com inicializador e o
bloco `static Foo() { ... }`. O compilador C# gera um único `.cctor` combinando inicializadores
estáticos em ordem textual e depois o corpo do `static` ctor (se houver).

### Dois regimes de disparo

O metadado do tipo carrega a flag `beforefieldinit` (ECMA‑335 §II.23.1.15). O C# a aplica
**quando a classe não declara um static ctor explícito** (somente inicializadores de campo);
com um `static Foo() {}` explícito, a flag **não** é emitida ("precise" semantics).

- **Sem `beforefieldinit` (precise)**: o `.cctor` roda **exatamente** antes do primeiro
  acesso a qualquer membro estático, ou antes da primeira criação de instância (o primeiro
  `newobj`), o que ocorrer primeiro.
- **Com `beforefieldinit` (relaxed)**: a inicialização pode rodar **a qualquer momento antes
  do primeiro acesso a um campo estático** do tipo. O JIT/AOT pode antecipá‑la (ex.: ao
  compilar o método que acessa o campo) ou adiá‑la até o acesso. Acessar um método estático
  ou construir uma instância **não** é gatilho obrigatório.

### Implementação no CoreCLR

- Flag no tipo: `IsBeforeFieldInit()` em `EEClass`; usado por exemplo em
  `src/coreclr/vm/methodtable.cpp` (~linha 4038), onde o runtime percorre a hierarquia e
  só dispara `CheckRunClassInitThrowing()` nos pais que **não** são `beforefieldinit`
  (~linha 4039), ao inicializar um tipo.
- Execução: `MethodTable::CheckRunClassInitThrowing` (methodtable.cpp ~linha 3871) e
  `MethodTable::RunClassInitEx` (~linha 3539). Há lock por tipo para tratar threads
  concorrentes: uma thread executa o `.cctor`, as demais esperam; um ciclo (A inicializa B
  que inicializa A na mesma thread) é permitido e vê o estado parcial (garantia ECMA, evita
  deadlock; threads diferentes em ciclo podem causar deadlock detectado e resolvido pelo
  runtime de forma específica).
- Falha: se o `.cctor` lança, o tipo fica marcado como falho e **toda** tentativa futura de
  usá-lo lança `TypeInitializationException` encapsulando a exceção original (referência a
  `kTypeInitializationException` em methodtable.cpp ~linhas 3659‑3685). O tipo nunca é
  reinicializado.
- Interface com o JIT: `corinfo.h` define `CORINFO_INITCLASS_NOT_REQUIRED`,
  `CORINFO_INITCLASS_INITIALIZED`, `CORINFO_INITCLASS_USE_HELPER`,
  `CORINFO_INITCLASS_DONT_INLINE` (linhas ~975‑979), respostas do runtime ao JIT sobre se
  precisa inserir uma chamada de helper (`CORINFO_HELP_INITCLASS`, linha ~365, ou helpers
  `GETSHARED_*STATIC_BASE`) no método que usa o tipo. Quando o tipo já está inicializado
  no momento da compilação JIT, o helper é omitido e o acesso a estático vira acesso direto
  a endereço; em Tier‑0→Tier‑1 isso é uma otimização central (por isso `static readonly`
  de tipos já inicializados vira constante).
- Em ReadyToRun/NativeAOT há casos de pré‑inicialização em compile‑time (NativeAOT executa
  alguns `.cctor` simples em tempo de compilação e embute os dados congelados no binário;
  veja `docs/design/coreclr/botr/ilc-architecture.md` e código em
  `src/coreclr/tools/aot/ILCompiler.Compiler/`).

### Armadilhas de ordem

- Ordem entre tipos diferentes não é definida (depende de quem é tocado primeiro).
- Ordem textual **dentro** de um tipo é determinística, mas entre arquivos de classes
  `partial` não é especificada.
- Dependências circulares de `static readonly` podem observar `null`/`0`.
- Genéricos: cada instanciação fechada (`Foo<int>`, `Foo<string>`) tem seu **próprio**
  conjunto de estáticos e seu próprio `.cctor` execução.

## Por quê

- **Lazy init** evita custo de startup: só paga o `.cctor` de tipos realmente usados.
- **`beforefieldinit`** existe para dar liberdade ao JIT: sem a garantia "precisa
  antes do primeiro acesso a qualquer membro", pode inicializar cedo, hoistar checagens para
  fora de loops, e eliminar checks de classe‑init em código quente. É por isso que o C#
  *só* marca beforefieldinit quando seguro (sem static ctor explícito = sem efeitos
  colaterais observáveis que a linguagem prometa ordenar).
- **Falha permanente**: simplicidade e segurança — estado parcialmente inicializado nunca é
  exposto como "válido" de novo.
- **Lock por tipo + thread‑safety**: o programador não escreve double‑checked locking; o
  runtime garante exatamente‑uma‑vez com visibilidade correta.

## Trade-offs

- Cada acesso estático em código não‑inicializado exige checagem/helper; mesmo após
  inicializar, código não‑otimizado paga o teste.
- Semântica precisa x relaxed é sutil, uma fonte comum de bugs ("meu static ctor roda
  cedo/tarde demais").
- Locks de inicialização podem causar deadlocks entre `.cctor`s concorrentes que dependem
  mutuamente de tipos.
- Exceções no `.cctor` “queimam” o tipo para o resto do processo — difícil recuperação.
- Isto prejudica o *startup*: muitos `.cctor` encadeados no caminho de inicialização
  (motivo de existir R2R/NativeAOT preinit).

## Como aplicar na sua linguagem

1. Escolha entre **inicialização eager** (ordem de import, estilo Go `init()`/Python
   módulo), **lazy por tipo** (C#/Java) ou **lazy por variável** (Swift globals com
   `dispatch_once`, Kotlin `object`). Lazy por tipo exige lock + estados
   (não‑inicializado/em‑andamento/pronto/falhou).
2. Se escolher lazy, decida se a garantia é **precisa** ou **relaxada** e deixe o programador
   optar (o .NET faz isso implicitamente pela presença do static ctor; uma flag/anotação
   explícita seria mais clara).
3. Defina o comportamento de **ciclos** (permitir e ver estado parcial, ou erro de
   compilação/runtime) e de **falha** (estado "poisoned" com exceção encapsulada, como
   `TypeInitializationException`).
4. Para AOT: avalie *pré‑inicializar* estáticos simples em compile‑time (heap congelado) —
   NativeAOT faz, reduz startup.
5. Exponha ao backend/JIT a pergunta “este tipo já está inicializado?” para eliminar checks
   (`CORINFO_INITCLASS_*`).
6. Genéricos reificados: isole estáticos por instanciação, ou documente se são compartilhados
   (como Java, por tipo erasure).

## Detalhes adicionais

### Exemplo prático dos dois regimes

```csharp
class A { public static readonly int X = Init("A.X"); }          // beforefieldinit
class B { public static readonly int X = Init("B.X");
          static B() { } }                                       // precise (sem beforefieldinit)
```

- `A`: o JIT/AOT pode rodar `Init("A.X")` antes de `Main` ou só quando `A.X` é lido; ambos
  são conformes.
- `B`: `Init("B.X")` roda exatamente antes do primeiro acesso a `B.X` ou da primeira
  criação de `B`.

Truque conhecido: adicionar um `static B() {}` vazio desativa `beforefieldinit` e força
semântica precisa (ao custo de checagens no código quente).

### Interação com Tiered Compilation

Em Tier-0 o JIT frequentemente ainda não vê o tipo inicializado e emite o helper de
inicialização. Quando o método é recompilado em Tier-1, o tipo já está inicializado e
`CORINFO_INITCLASS_INITIALIZED` permite remover o helper e embutir o endereço dos
estáticos (e, para `static readonly`, até os valores). Isto explica por que
`static readonly` importa para desempenho no .NET (comportamento de otimização conhecido; a
interface relevante é `initClass` em `corinfo.h`).

### Estados de inicialização

O runtime mantém por tipo um estado de "classe inicializada" nos flags do `MethodTable`
(`IsInitedIfStaticDataAllocated` em `methodtable.h` linha ~1236) e um lock de inicialização
por tipo para serializar a execução e tratar ciclos.

### Perguntas de projeto a responder na sua linguagem

- O inicializador pode observar estado parcial de outro tipo em ciclo? (.NET: sim.)
- A falha é permanente? (.NET: sim, `TypeInitializationException`.)
- Inicialização é por tipo, por módulo ou por variável?
- O backend pode antecipar a inicialização? (.NET: só com `beforefieldinit`.)
- Há suporte a pré-computar estáticos em compile-time? (NativeAOT: `TypePreinit.cs` e
  `PreinitializationManager.cs` em `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/`.)
- Qual a regra para genéricos: um conjunto de estáticos por instanciação fechada?

## Referências

- `src/coreclr/vm/methodtable.cpp` — `RunClassInitEx`, `CheckRunClassInitThrowing`, uso de
  `IsBeforeFieldInit`, criação de `TypeInitializationException`.
- `src/coreclr/inc/corinfo.h` — `CORINFO_INITCLASS_*`, `CORINFO_HELP_INITCLASS`.
- `src/coreclr/vm/methodtable.h` — `IsInitedIfStaticDataAllocated`.
- `docs/design/coreclr/botr/ilc-architecture.md` — pré‑inicialização no NativeAOT.
- ECMA‑335 §II.10.5.3 (type initializer) e §II.23.1.15 (`beforefieldinit`) — conhecimento
  externo ao repo.
