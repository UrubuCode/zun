# newobj e construtores de instância

## Mecanismo no .NET

### O opcode IL `newobj`

Em C#, `new Foo(args)` para um tipo de referência é compilado para a instrução IL
`newobj instance void Foo::.ctor(args)`. O opcode faz **duas coisas atômicas do ponto de
vista do IL**:

1. Aloca uma instância nova do tipo, com todos os campos zerados.
2. Chama o construtor `.ctor` indicado, passando a referência recém-alocada como `this`
   (argumento 0) e os argumentos da pilha de avaliação; ao final empilha a referência.

Isto difere de `newarr` (arrays), `initobj` (struct in-place) e `ldelema`/`stackalloc`.
Para value types `newobj` também existe: o compilador aloca um temporário na pilha do método,
chama o `.ctor` sobre o endereço dele e copia o valor (o JIT normalmente elimina a cópia).

O construtor de instância é um método com nome especial `.ctor`, flags `rtspecialname` e
`specialname`; o construtor estático é `.cctor` (arquivo 04).

### Como o runtime executa

Caminho (CoreCLR), em alto nível, confirmado pela estrutura do código:

- O JIT pede ao runtime o helper de alocação apropriado via `getNewHelper`. O enum de helpers
  está em `src/coreclr/inc/corinfo.h` (por exemplo `CORINFO_HELP_NEWSFAST`, descrito no
  próprio arquivo como "allocator for small, non-finalizer, non-array object").
  Existem variantes para objetos finalizáveis, com alinhamento 8, arrays etc.
- Os helpers rápidos chamam em último caso o alocador gerenciado do VM em
  `src/coreclr/vm/gchelpers.cpp` (`AllocateObject`, `AllocateSzArray`, `Alloc`), que por sua vez
  chama `GCHeapUtilities::GetGCHeap()->Alloc(pAllocContext, size, flags)` (linhas ~270-380 do
  arquivo). A interface do GC está em `src/coreclr/gc/gcinterface.h`.
- No Native AOT, o equivalente é `RhpNewFast` em `src/coreclr/nativeaot/Runtime/portable.cpp`
  (com variantes `RhpNewFastAlign8` e `RhpNewFastMisalign`).
- Após a alocação, o JIT emite uma chamada normal ao `.ctor` (inlining possível). O código do
  construtor roda como qualquer método de instância.

O objeto retornado pelo helper já tem **header zerado**, **ponteiro de MethodTable
preenchido** e **campos zerados** (veja arquivos 02 e 06). Portanto o runtime garante que
todo construtor observa o objeto no estado "default" para todos os campos, antes de qualquer
código do usuário.

### Regras do construtor

- Um construtor de classe deve (segundo as regras do C# / verificação) chamar um construtor da
  classe base ou outro construtor da mesma classe antes de usar `this`; o compilador injeta
  `call Base::.ctor()` automaticamente quando ausente. O runtime em si **não exige** isto no
  IL (a verificação de IL é opcional), mas o C# sempre o gera.
- Construtores não são virtuais; `newobj` não faz despacho virtual do `.ctor`. Já chamar um
  método virtual de dentro do construtor despacha pelo tipo **real** (MethodTable já é o do
  tipo mais derivado) — armadilha clássica: o método derivado roda antes do construtor
  derivado (arquivo 03).
- Se o construtor lança, o objeto já alocado simplesmente fica inalcançável e será coletado;
  se o tipo tem finalizador, o finalizador **ainda roda** (o objeto foi registrado para
  finalização na alocação, não ao fim do ctor) — ver arquivo 08. Isto é confirmado pelo
  fato de o registro na fila de finalização ocorrer no momento da alocação (flags de
  alocação em `gchelpers.cpp`; `CFinalize::RegisterForFinalization` em
  `src/coreclr/gc/finalization.cpp`).

### Value types

Para structs `new S(args)` gera `newobj` com `.ctor` sobre um local; `new S()` sem
argumentos em C# 10+ chama o ctor sem parâmetros declarado pelo usuário se existir, senão é
`initobj` (zera). Detalhes no arquivo 09.

### Outras formas de criar instância

- `Activator.CreateInstance` / `RuntimeHelpers.GetUninitializedObject`: alocam sem rodar
  ctor (o segundo explicitamente).
- `MemberwiseClone`: copia sem rodar ctor.
- Desserialização / `RuntimeHelpers.GetUninitializedObject`: idem.
- Delegates e arrays têm criação especial pelo runtime (não via `newobj` comum).
- Reflection (`ConstructorInfo.Invoke`) chega ao mesmo caminho de alocação + chamada.

## Por quê

- **Atomicidade conceitual**: juntar alocação + construção num opcode permite ao
  verificador garantir que ninguém use a referência sem ter passado por um ctor.
- **Zeragem garantida**: remove classe inteira de bugs (memória não inicializada); é o
  contrato de segurança de memória do CLR.
- **Separação alocação/inicialização**: o runtime/GC cuida de memória e metadados do objeto;
  a linguagem cuida da lógica de inicialização — permite trocar o GC (o GC é plugável via
  `gcinterface.h`) sem tocar na semântica de construtores.
- **Helpers especializados**: um helper por classe de objeto (pequeno, finalizável, array,
  alinhamento) tira ramificações do caminho quente.

## Trade-offs

- Construtor rodando com `this` já visível: o objeto pode "escapar" (ser publicado em campo
  estático, passado a outra thread, chamar virtual) antes de totalmente construído. Linguagens
  como Swift (two-phase init) e Kotlin avisam sobre "leaking this"; C# deixa passar.
- Zeragem obrigatória custa banda de memória (mitigada por zerar em blocos no allocation
  context e por `GC.AllocateUninitializedArray`, arquivo 06/10).
- `newobj` ser opcode e não chamada de função acopla o IL ao modelo de objetos; é mais difícil
  uma segunda linguagem expressar "construção fábrica" sem açúcar.
- Falha parcial: objetos de ctor que lançaram ainda podem ser finalizados em estado
  inconsistente.

## Como aplicar na sua linguagem

1. Defina formalmente a **semântica de construção**: (a) memória zerada, (b) ordem de chamada
   de ctors base→derivado, (c) quando `this` pode ser usado. Escreva isso na especificação.
2. Considere decidir se `this` pode escapar do construtor. Opções: proibir (estilo Rust/Swift
   com fases), permitir com aviso (C#), ou permitir tudo (Java).
3. Implemente a construção no IR como duas operações (`alloc`, `call ctor`) para permitir ao
   otimizador fazer inlining do ctor e eliminar alocação por escape analysis. O .NET mantém
   `newobj` único, mas o JIT o decompõe internamente (helper + call).
4. Ofereça um **helper de alocação por categoria** (pequeno/grande/finalizável/array) para
   manter o fast path curto.
5. Se tiver finalizadores, defina quando o objeto é registrado (na alocação, como .NET, ou ao
   fim do ctor, como algumas VMs preferem para evitar finalizar objetos pela metade).
6. Forneça uma API de "alocar sem construir" (equivalente a `GetUninitializedObject`) apenas
   para runtime/serialização e marque-a como unsafe.

## Detalhes adicionais

### Exemplo: IL de uma criação simples

```
// C#:  var p = new Point3D(1, 2, 3);   (class Point3D)
newobj  instance void Point3D::.ctor(int32, int32, int32)   // com 1,2,3 já na pilha
stloc.0
```

O JIT, via `getNewHelper`, resolve o `newobj` em: (1) chamada ao helper de alocação
(`CORINFO_HELP_NEWSFAST` ou variante), (2) chamada (ou inline) do `.ctor` com o resultado
como `this`. Se o tipo é finalizável o helper muda (registro na fila de finalização), se o
tamanho é grande a alocação cai no LOH (arquivo 07).

### Checklist de decisões para o designer

| Decisão | Opções vistas | Escolha do .NET |
|---|---|---|
| Alocação e ctor | fundidas ou separadas no IR | fundidas no opcode, separadas no JIT |
| Estado antes do ctor | indefinido / zero | sempre zero |
| Tipo do objeto durante ctor | muda por nível (C++) / final | final (MethodTable derivado) |
| Registro de finalização | início / fim do ctor | na alocação |
| Falha no ctor | desfaz / abandona | abandona; GC coleta |
| Ctor virtual | permitido / não | não existe |

### Observações adicionais

- Em Native AOT o caminho é equivalente mas sem JIT: o compilador ILC emite chamadas para
  `RhpNewFast` e variantes diretamente. `src/coreclr/vm/gchelpers.cpp` (linha ~57) contém
  `RhpGcAlloc(MethodTable*, GC_ALLOC_FLAGS, intptr_t numElements, ...)`, que escolhe entre
  `AllocateSzArray` e `AllocateObject`.
- Arrays são criados por `newarr` (ou helpers de array multidimensional), não por `newobj`.
- Strings têm helper próprio porque o tamanho depende do comprimento.
- Existir um único ponto (`Alloc` em gchelpers) para todas as alocações facilita
  instrumentação: `LogAlloc` (gchelpers.cpp ~linha 417), eventos de alocação e profilers.

## Referências

- `src/coreclr/inc/corinfo.h` — enum de helpers `CORINFO_HELP_NEW*`.
- `src/coreclr/vm/gchelpers.cpp`, `gchelpers.h`, `gchelpers.inl` — alocação no VM.
- `src/coreclr/gc/gcinterface.h` — interface `IGCHeap::Alloc`.
- `src/coreclr/gc/finalization.cpp` — `CFinalize::RegisterForFinalization`.
- `src/coreclr/nativeaot/Runtime/portable.cpp` — `RhpNewFast*` (Native AOT).
- `docs/design/coreclr/botr/garbage-collection.md` — seção de alocação.
- ECMA-335, partição III, `newobj` (não incluída no repo; citação por conhecimento da
  especificação).
