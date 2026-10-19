# Ordem de inicialização de campos e corpo do construtor

## Mecanismo no .NET

### A ordem em C# (compilador Roslyn)

Para uma classe, o `.ctor` gerado pelo compilador C# tem a seguinte sequência:

1. **Inicializadores de campo de instância** da classe atual, na ordem textual
   (`int x = 5;`, `List<int> l = new();`).
2. Chamada ao **construtor base** (`call Base::.ctor`) — ou ao outro ctor da mesma classe
   via `: this(...)`.
3. O **corpo** do construtor.

Detalhe importante e específico do C#: os inicializadores de campo rodam **antes** do
construtor base, ao contrário de Java (onde rodam depois do `super()`). Consequência: em

```csharp
class B { public B() { Init(); } protected virtual void Init() { } }
class D : B {
    int x = 42;
    string s = "a";
    protected override void Init() { Console.WriteLine(x + s); }  // x==42, s=="a"
    public D() : base() { }
}
```

`Init()` (chamado pelo ctor base, despacho virtual para `D.Init`) vê `x == 42`, pois o
inicializador de campo já rodou. Em Java o equivalente veria `x == 0`. Os campos **não
declarados com inicializador** ainda são default (zero/null), porque o runtime zerou o
objeto na alocação (arquivos 01 e 06).

Se o construtor encadeia com `: this(...)`, os inicializadores de campo são emitidos
apenas no ctor que chama `base`, não duplicados.

Esta descrição é do comportamento da linguagem C# (especificação C#) e do compilador Roslyn.
O repositório `dotnet/runtime` não contém o compilador C# (Roslyn está em `dotnet/roslyn`),
portanto o que está acima é conhecimento da especificação, não verificado em código deste
repo. O que é verificável aqui é o lado do runtime: o objeto já está zerado e com
MethodTable do tipo mais derivado quando o primeiro `.ctor` (o mais derivado) começa, e
ctors base são chamadas normais.

### O que o runtime garante

- Antes de qualquer `.ctor` executar, todos os campos são zero/null (alocação do GC zera a
  memória; ver `gc_alloc_context` em `src/coreclr/gc/gcinterface.h` e o texto sobre
  "allocation quantum" em `docs/design/coreclr/botr/garbage-collection.md`).
- O MethodTable do objeto é o do **tipo concreto** desde a alocação, de modo que chamadas
  virtuais de dentro de um ctor base despacham para o override do tipo derivado — mesmo que
  o ctor derivado ainda não tenha rodado.
- Não há "fase de inicialização" separada no runtime: o objeto é plenamente válido para o
  GC mesmo no meio do ctor (todos os campos de referência são null ou apontam para objetos
  válidos). Isto é essencial: o GC pode ocorrer a qualquer momento dentro do ctor.

### Campos `readonly` e `init`

- Campos `readonly` só podem ser atribuídos em inicializador ou ctor (checagem do
  compilador); o runtime/JIT **não impõe** em runtime (reflexão pode alterar, salvo `static
  readonly` após inicialização em alguns cenários de JIT onde o valor é tratado como
  constante — o JIT pode embutir `static readonly` já inicializados em código Tier‑1).
- Propriedades `init` (C# 9) compilam para setters com modificador `modreq(IsExternalInit)`;
  só são permitidas em inicializadores de objeto e ctors (ver arquivo 05).

### Modelo de memória e publicação

O .NET não garante, por si só, que escritas no ctor sejam visíveis a outra thread que obtenha
a referência por data race. O modelo de memória do CLR dá semântica de *release* às escritas
de referência (stores são release no CoreCLR) em x86/x64; em ARM64 o runtime insere barreiras
na publicação via write barrier/`Volatile`. Campos `readonly`/`final`-like não têm a
garantia especial do Java. (Descrição geral do modelo de memória; ver
`docs/design/specs/Memory-model.md` neste repo para detalhes.)

### Structs

Em structs, não há ctor base; campos sem inicializador devem ser atribuídos antes do ctor
retornar (definite assignment) em versões antigas do C#; a partir do C# 11 os campos são
auto‑default. Ver arquivo 09.

## Por quê

- **Campos inicializados antes do base ctor (C#)**: garante que métodos virtuais chamados
  durante a construção do base nunca vejam referências *null* de campos que têm
  inicializador (reduz NullReferenceException). Foi uma escolha consciente da equipe do C#.
- **Zero‑fill pelo runtime**: simplifica o compilador (não precisa emitir stores de zero) e é
  um requisito de segurança.
- **MethodTable final desde o início**: evita um objeto mudar de tipo (como em C++, onde o
  vptr muda a cada ctor da cadeia), simplificando o runtime e o GC, ao custo da armadilha da
  chamada virtual no construtor.
- **Ordem textual determinística** dos inicializadores facilita raciocínio e debugging.

## Trade-offs

- Chamar virtual no ctor expõe estado parcialmente construído (campos sem inicializador
  ainda default, ctor derivado ainda não rodou). Analisadores (CA2214) alertam.
- Inicializadores antes do base ctor significam que **não podem referenciar `this`** nem
  parâmetros do ctor (erro do compilador) – limitação ergonômica.
- Java/C++ escolhem o oposto (base primeiro, campos depois), o que torna virtuais mais
  seguros em campos mas permite ver campos zerados.
- Sem fases (two-phase init) o compilador não pode provar "totalmente inicializado" para
  tipos não‑nulos: C# precisa de `required` (C# 11) e anotações de nulabilidade como
  tratamento parcial.

## Como aplicar na sua linguagem

1. **Especifique a ordem** em uma tabela: (a) zerar, (b) inicializadores de campo, (c) ctor
   base, (d) corpo. Escolher campos-antes-do-base (C#) ou base-antes-de-campos (Java) tem
   consequências em virtuais; documente.
2. Se oferecer tipos não‑nulos, exija **definite assignment** de todos os campos não‑nulos
   antes de `this` escapar (Swift, Kotlin com `lateinit`/avisos, Dart `late`).
3. Proíba ou avise sobre chamar métodos virtuais e passar `this` a outros dentro do ctor.
4. Aproveite alocação com zero‑fill para que o objeto seja sempre seguro ao GC; assim o
   coletor não precisa saber de "objeto em construção".
5. Considere `required`/construtores nomeados/fábricas como alternativa a objetos
   parcialmente válidos.
6. Se tiver `readonly`, decida se o JIT/AOT pode assumir imutabilidade pós‑ctor para
   otimizações (constant folding). O .NET faz isso para `static readonly`.
7. Defina o modelo de memória de publicação segura (ex.: stores no ctor têm release
   implícito antes de publicar a referência) — o .NET dá isso na prática, mas não por
   especificação forte para `readonly`.

## Detalhes adicionais

### Exemplo comparativo de ordens

| Linguagem | 1º | 2º | 3º | Virtual no ctor base vê campos do derivado? |
|---|---|---|---|---|
| C# | inits de campo do derivado | ctor base | corpo do derivado | sim, com inicializadores já aplicados |
| Java | ctor base | inits de campo do derivado | corpo do derivado | vê default (0/null) |
| C++ | ctor base | inits (lista de membros) | corpo | não: vptr ainda é da base |
| Swift | inits do derivado | `super.init` | resto | fases rígidas, proíbe uso de self antes |

Fonte das linhas de outras linguagens: conhecimento geral das especificações; não verificado
neste repositório.

### Como observar no IL

O IL de `class D : B { int x = 42; public D() { Foo(); } }` tem a forma:

```
.method public instance void .ctor() {
  ldarg.0
  ldc.i4.s 42
  stfld int32 D::x            // inicializador de campo
  ldarg.0
  call instance void B::.ctor()   // base
  ldarg.0
  call instance void D::Foo()     // corpo
  ret
}
```

### Implicações para o GC e o JIT

- O JIT trata `stfld` de referência no ctor com **write barrier** normal; para objetos
  recém alocados em gen0 a barreira é barata, mas ainda executa.
- Ctors triviais (só chamam `object::.ctor`) são inlinados e eliminados.
- Se o tipo tem finalizador e o ctor lança antes de atribuir recursos, o finalizador precisa
  tolerar campos nulos.

## Referências

- Especificação C# (§ Instance constructors, Variable initializers) — conhecimento externo
  ao repo; Roslyn em `dotnet/roslyn`.
- `src/coreclr/gc/gcinterface.h` (`gc_alloc_context`).
- `docs/design/coreclr/botr/garbage-collection.md` (allocation contexts / zeragem).
- `docs/design/specs/Memory-model.md` (modelo de memória).
- `src/coreclr/vm/object.h` (Object, MethodTable já definido na alocação).
