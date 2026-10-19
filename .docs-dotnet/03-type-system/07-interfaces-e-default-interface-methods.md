# Interfaces e default interface methods (DIM)

> Fonte principal: seção "Default Interface Methods" de `docs/design/specs/Ecma-335-Augments.md`
> (o arquivo `docs/design/features/default-interface-methods.md` apenas redireciona para ela:
> "The details previously held in this document have been moved to Ecma-335-Augments").

## Mecanismo no .NET

### 1. Interface como categoria de tipo

Interfaces são `MethodTable`s com categoria própria (`enum_flag_Category_Interface`,
`src/coreclr/vm/methodtable.h` ~linha 1431). Um tipo concreto mantém uma *interface map* (tabela de
interfaces implementadas) e, por interface, um mapeamento slot-da-interface → implementação.
`MethodTableBuilder` (`src/coreclr/vm/methodtablebuilder.cpp`) constrói a tabela e resolve
`MethodImpl`s.

Chamada por interface não usa vtable direta: usa **virtual stub dispatch**
(`docs/design/coreclr/botr/virtual-stub-dispatch.md`; implementação em
`src/coreclr/vm/virtualcallstub.cpp`).

### 2. O que o DIM muda (ECMA-335 Augments)

Principais mudanças listadas no documento:
- Interfaces passam a poder ter **métodos de instância** (virtuais e não virtuais). Antes só
  existiam métodos virtuais abstratos.
- Interfaces **continuam sem campos de instância**.
- Um método de interface pode fazer `MethodImpl` de outro método de uma interface que ele
  *requer* (herda), desde que o `MethodImpl` seja `final`. Uma interface pode fornecer/substituir a
  implementação de método de outra.
- Regras da spec: todo método virtual de instância numa interface deve ser (a) abstrato, `newslot`,
  sem `MethodImpl` que o use como Impl, ou (b) `final` sem `newslot` e com `MethodImpl` que o use
  como Impl.
- Acessibilidade: interfaces podem ter membros `protected`/`internal`; "referentes que suportam o
  mesmo tipo" passa a incluir tipos que *implementam* a interface.
- Type initializers: o runtime sempre rodou `.cctor` ao acessar estáticos de interface; DIM
  atualiza a semântica de quando métodos de instância de interface disparam o cctor.

### 3. Algoritmo de resolução (a parte que importa)

DIM é **um fallback**: "default interface methods ... é sempre usado como fallback — só se as
'regras antigas' não encontrarem implementação, aplicamos as novas regras" (texto do documento).
Construção da tabela de interfaces na forma aberta permanece, exceto que o erro "classe não abstrata
com slot vazio" só ocorre se o método definidor *não for abstrato* e não houver `MethodImpl` nas
interfaces.

Resolução em tempo de chamada:
1. Crie uma lista vazia de candidatos.
2. Se o método de interface *não é abstrato*, adicione-o.
3. Aplique todos os `MethodImpl`s das interfaces implementadas implicitamente pela classe de runtime
   do objeto e adicione-os.
4. Para cada candidato, se o tipo dono é "menos concreto" que o de outro candidato (o outro dono
   *requer* o primeiro), remova-o.
5. Mais de um restante → `AmbiguousImplementationException`.
6. Exatamente um e não abstrato → chame-o.
7. Exatamente um e abstrato → `EntryPointNotFoundException`.
8. Nenhum e a interface é variante → repita procurando correspondência variante, devolvendo "a
   primeira correspondência fornecida por uma interface mais específica".

Também `callvirt` e `ldvirtftn` ganham as exceções `AmbiguousImplementationException` /
`EntryPointNotFoundException`.

### 4. Interação com value types

O texto de `constrained.` (III.2.1) é estendido: "No caso em que a implementação do método da
interface é fornecida por uma interface, o **boxing implícito torna-se observável** ao programa".
Isto é, chamar um DIM sobre um struct exige que `this` seja um objeto boxed (a interface não conhece
o layout do struct). Ver `06`. Em `byreflike-generics.md`, ByRefLike com `constrained. callvirt`
lança `NotSupportedException` se o alvo resolver a DIM.

### 5. Static virtual / abstract members em interfaces

O mesmo Augments (~linhas 500-760) adiciona **membros estáticos virtuais** em interfaces, resolvidos
por tipo concreto em tempo de execução ("Interfaces may define static virtual methods that get
resolved at runtime based on actual types involved"). O `.override` passa a poder implementar método
virtual com método estático não virtual. É o que habilita interfaces como `INumber<T>` usadas em
constraints genéricas (ver `04`).

### 6. Interfaces genéricas e variância

Interfaces são o principal veículo da variância (ver `05`). O algoritmo DIM prevê o caso variante
(passo 8). `CanCastByVarianceToInterfaceOrDelegate` em `methodtable.cpp` é o teste de
compatibilidade.

### 7. Covariant returns (não se aplica a interfaces)

O documento de retorno covariante informa que só se aplica a métodos de **classes** (não interfaces)
(`Ecma-335-Augments.md` ~linha 850).

## Por quê

- **Evolução de interfaces sem quebrar implementadores** (motivação principal do recurso C# 8,
  referenciado no documento; "traits"-like).
- **Reuso por composição** sem herança múltipla de estado: a restrição de "sem campos de instância"
  evita o problema do diamante de layout.
- **Resolução determinística** pelo critério "mais específico"; ambiguidade vira erro de runtime
  explícito em vez de escolha arbitrária.
- **Design como fallback** preserva o comportamento de código existente: quem já implementava a
  interface continua resolvendo como antes.

## Trade-offs

- **Ambiguidade só aparece em runtime** (`AmbiguousImplementationException`): o compilador tenta
  prevenir, mas um assembly recompilado separadamente pode criar diamantes.
- **Boxing observável** em structs que usam DIM.
- **Custo no carregador e no dispatch**: o passo 8 e a coleta de candidatos exigem varrer interfaces;
  o resultado costuma ser cacheado pelo stub de dispatch (inferência a partir da arquitetura VSD).
- **Modelo mental dividido**: DIM não substitui classe abstrata (sem estado), e acessar membro DIM
  exige tratar como interface (`((IFoo)obj).M()`, regra do C#, conhecimento geral).
- **Mais superfície de compatibilidade**: toda extensão (static virtual, DIM) toca em IL, metadado,
  JIT, trimmer e bibliotecas.

## Como aplicar na sua linguagem

1. **Decida se interfaces têm corpo.** DIM dá evolução de API, mas exige regra de ambiguidade e
   runtime que a resolva. Alternativas: extension methods (resolvidos estaticamente) ou mixins.
2. **Proíba estado em interfaces** (como o .NET), a menos que aceite herança múltipla com layout
   complexo.
3. **Defina "mais específica" formalmente**: o .NET usa "o dono de um candidato *requer* o dono do
   outro". Reuse essa definição.
4. **Trate o diamante como erro explícito** (com mensagem clara), não resolva por ordem textual.
5. **Faça DIM um fallback**: primeiro as implementações de classe (regras antigas), depois as
   interfaces — mantém compatibilidade binária.
6. **Planeje membros estáticos abstratos** se quiser generic math/abstração sobre operadores:
   resolva por tipo concreto na instanciação.
7. **Defina interação com value types**: documente quando há boxing ao chamar um DIM.
8. **Use despacho com cache** (stub/inline cache) para chamadas por interface; é onde o custo mora.

### Exemplo: resolução em diamante

```csharp
interface IA { void M() => Console.WriteLine("IA"); }
interface IB : IA { void IA.M() => Console.WriteLine("IB"); }   // MethodImpl em IA.M
interface IC : IA { void IA.M() => Console.WriteLine("IC"); }
class D : IB, IC { }
((IA)new D()).M();   // candidatos: IB.M e IC.M; nenhum dono "requer" o outro -> AmbiguousImplementationException
```

Se `D : IB` apenas, o candidato é `IB.M` (mais específico que o `IA.M` original, porque `IB`
requer `IA`), então `IA.M` é removido da lista e `IB.M` vence.

### Checklist de decisões

- [ ] Interfaces têm corpo? Estado? Estáticos abstratos?
- [ ] Critério formal de "mais específico"?
- [ ] O que acontece em ambiguidade (erro de compilação, runtime ou regra de desempate)?
- [ ] DIM é fallback sobre as implementações das classes?
- [ ] Como o custo do dispatch por interface é amortizado (stubs, caches)?

## Referências

- `docs/design/specs/Ecma-335-Augments.md` (seções "Default Interface Methods", "Static virtual members", "Covariant Return Types")
- `docs/design/features/default-interface-methods.md` (redireciona ao Augments)
- `docs/design/coreclr/botr/virtual-stub-dispatch.md`
- `src/coreclr/vm/virtualcallstub.cpp`
- `src/coreclr/vm/methodtablebuilder.cpp`
- `src/coreclr/vm/methodtable.h`, `src/coreclr/vm/methodtable.cpp`
- `docs/design/features/byreflike-generics.md`
