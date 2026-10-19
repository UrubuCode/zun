# Virtual dispatch e MethodTable/vtable

Convenção: "(repo)" = verificado lendo arquivos deste repositório. "(inferência)" = dedução minha a partir do código, não declarada pelos autores.

## Mecanismo no .NET

### Quem é quem: MethodTable, EEClass, MethodDesc

- `MethodTable` (`src/coreclr/vm/methodtable.h`, `class MethodTable` na linha ~978): estrutura "quente" por tipo. Todo objeto no heap começa com um ponteiro para o `MethodTable` do seu tipo exato (o "type handle" do objeto). Contém: ponteiro para o pai (`m_pParentMethodTable`), número de interfaces (`m_wNumInterfaces`), vtable em chunks, mapa de interfaces, dispatch map opcional (`GetDispatchMap()`), flags de categoria (`enum_flag_Category_*`: classe, interface, array, nullable...) (repo).
- `EEClass` (`src/coreclr/vm/class.h`): dados "frios" do tipo, usados só em carga de tipo/reflexão (repo; não aprofundei neste arquivo).
- `MethodDesc` (`src/coreclr/vm/method.hpp`): descritor de cada método. Ver `docs/design/coreclr/botr/method-descriptor.md` (repo): todo MethodDesc tem um "slot" que guarda o entry point atual; o slot fica no `MethodTable` para métodos que precisam de lookup por índice (virtuais, métodos de tipos genéricos) e dentro do próprio MethodDesc caso contrário.

### A vtable é de dois níveis

O comentário em `methodtable.h` (linhas ~1662-1710, "The VTABLE") diz textualmente:

> "Rather than the traditional array of code pointers (or "slots") we use a two-level vtable in which slots for virtual methods live in chunks. Doing so allows the chunks to be shared among method tables (the most common example being between parent and child classes where the child does not override any method in the chunk). This yields substantial space savings at the fixed cost of one additional indirection for a virtual call."

Detalhes confirmados (repo):

- `#define VTABLE_SLOTS_PER_CHUNK 8` e `VTABLE_SLOTS_PER_CHUNK_LOG2 3`.
- O MethodTable tem "indirection slots" (ponteiros para chunks). `GetVtableIndirections()`, `GetIndexOfVtableIndirection(slot)` e `GetIndexAfterVtableIndirection(slot)` convertem número de slot em (chunk, offset).
- Um filho que não sobrescreve nada num chunk *compartilha* o chunk do pai.
- O comentário menciona uma estratégia testada de começar com um chunk de tamanho 4, "matching the number of virtuals in System.Object", mas rejeitada por ganho pequeno e custo nos helpers.
- Apesar disso, "all other code continues to refer to a virtual method via the traditional slot number".

Acesso a um slot virtual (`MethodTable::GetSlotForVirtual`, ~linha 1627):

```
GetVtableIndirections()[GetIndexOfVtableIndirection(slotNum)] + GetIndexAfterVtableIndirection(slotNum)
```

Ou seja: `mt -> indirections[slot >> 3] -> chunk[slot & 7] -> código`. Duas cargas dependentes mais o salto.

Métodos não virtuais "com slot de vtable" (`GetNumVirtuals() <= slot < GetNumVtableSlots()`) ficam num array separado que "cresce para trás" a partir de `MethodTableAuxiliaryData` (`GetSlotPtrRaw`, ~linha 1639-1651) (repo).

### Quantos virtuais, quais são herdados

- `GetNumVirtuals()` (~1780): total de slots virtuais do tipo.
- `GetNumParentVirtuals()` (~1793): virtuais do pai; interfaces retornam 0.
- `GetNumVtableSlots()` (~1817): virtuais + não virtuais com slot.
- `NO_SLOT = 0xffff`: sentinela de "sem slot".
- `GetRestoredSlot` / `GetRestoredSlotMT` (~1839-1842): obtém o valor do slot resolvendo de qual MethodTable ele realmente vem (por causa do compartilhamento) (repo).

### Como o type loader decide slots (override vs novo slot)

Em `src/coreclr/vm/methodtablebuilder.cpp`, ao enumerar os métodos de um tipo (região de ~5380-5440):

- Se o método tem `newslot` (`IsMdNewSlot`), ele *não procura* no pai: ganha slot novo.
- Se não tem `newslot` e há pai, chama `LoaderFindMethodInParentClass` para achar um método com mesmo nome e assinatura. Se achar:
  - pai não virtual: erro (`BFA_NONVIRT_NO_SEARCH`);
  - pai `final`: erro (`IDS_CLASSLOAD_MI_FINAL_DECL`);
  - senão, `TestOverRide` e o método reutiliza o slot do pai (override).

Também existe o caminho explícito por `MethodImpl` (`.override` do IL), com validações em ~4870-4910: o método declarado deve ser virtual, não `final`, etc. Ver arquivo 04.

### Chamada na prática

Em IL, `callvirt` sobre método virtual de classe vira, no código JIT, algo como: carregar o MethodTable do objeto, ler o chunk, ler o slot, chamar. `Object` tem slots hardcoded: em `src/libraries/System.Private.CoreLib/src/System/Object.cs` o comentário no finalizador diz "This method's virtual slot number is hardcoded in runtimes. Do not add any virtual methods ahead of this." (repo). Isso mostra um acoplamento forte: o runtime nativo conhece o índice de `Finalize` em `System.Object`.

### E o Virtual Stub Dispatch para virtuais de classe?

`docs/design/coreclr/botr/virtual-stub-dispatch.md` (seção "Current State") afirma: VSD "is enabled only for interface method calls but not virtual instance method calls", pelos motivos de (1) startup (muitos stubs iniciais) e (2) throughput ("virtual instance method calls suffer an unacceptable speed degradation"). Consequência: "every type has a vtable for virtual instance methods" (repo). Ou seja, virtuais de classe = vtable; interfaces = stubs/dispatch maps (arquivo 03).

### Atualização de slot e tiered compilation

O slot guarda o entry point *atual*. Com compilação em camadas e backpatching de slots, o runtime tem estruturas auxiliares como `src/coreclr/vm/methoddescbackpatchinfo.h/.cpp` para repatchar slots quando o código do método é substituído (existência do arquivo confirmada; detalhes da política não verificados por mim). Design geral: `docs/design/features/tiered-compilation.md` e `code-versioning.md` (repo; não li em profundidade).

## Por quê

Razões que o código e os docs sustentam:

1. **Chamada virtual O(1) e previsível.** Com herança única, o índice do slot é fixo para toda a hierarquia, então o JIT emite carga-indexada sem conhecer o tipo exato.
2. **Economia de memória via chunks compartilhados.** Declarado no comentário de `methodtable.h`: troca uma indireção a mais por substancial economia de espaço (pais e filhos compartilham chunks).
3. **Slot = entry point mutável.** Como todo MethodDesc tem slot (method-descriptor.md), o runtime pode redirecionar chamadas (stubs de pré-código, JIT tardio, tiering) sem reescrever call sites que usam vtable.
4. **Separar virtual de classe e de interface.** O doc de VSD explica que a otimização de stubs trouxe ganho para interfaces, mas regressão para virtuais de classe; por isso são mecanismos diferentes.

## Trade-offs

- **Indireção extra por chunking**: custo fixo de uma carga a mais em toda chamada virtual, em troca de memória (declarado).
- **Tamanho de vtable cresce com a hierarquia**: cada virtual novo adiciona slot em todos os descendentes (a menos que compartilhem chunks).
- **Fragile base class**: inserir um virtual no meio de uma base muda índices dos descendentes. O .NET mitiga em ReadyToRun com mecanismos de versionamento (`docs/design/coreclr/botr/readytorun-overview.md`, repo; detalhes não verificados aqui), e `Object` tem a restrição explícita de slots.
- **Devirtualização/inlining**: virtual impede inlining sem informação extra (sealed, tipo exato conhecido, PGO). Este arquivo não detalha o JIT; ver `src/coreclr/jit/` (inferência de que há otimizações de devirtualização, não verificado aqui).
- **Dois mecanismos** (vtable + dispatch map) dobram a superfície de manutenção do runtime.

## Como aplicar na sua linguagem

1. **Se tem herança única, use vtable linear.** Slot herdado mantém o índice; override reutiliza o índice; método novo recebe o próximo.
2. **Decida o formato de slot**: array simples (uma indireção a menos) ou chunked (menos memória por tipo). Para linguagens com muitas classes pequenas e hierarquias profundas, o compartilhamento do .NET pode compensar; para poucas classes, array simples é mais rápido e mais simples.
3. **Faça o slot ser um ponteiro mutável**, para permitir JIT tardio, tiers e hot reload sem recompilar call sites.
4. **Reserve slots conhecidos** (ex.: destrutor, `toString`, `equals`, `hash`) em posições fixas da raiz, como o .NET faz com `Object`. Documente que a raiz não pode ganhar virtuais "na frente".
5. **Defina a regra de override**: por nome+assinatura (como no .NET quando não há `newslot`) ou por marca explícita. O .NET separa "override implícito" de "novo slot" via o bit `newslot` do método. Uma alternativa é exigir `override` explícito e erro caso contrário.
6. **Verifique na carga do tipo**: pai virtual? pai final? assinaturas compatíveis? O `MethodTableBuilder` faz isso (erros `BFA_NONVIRT_NO_SEARCH`, `IDS_CLASSLOAD_MI_FINAL_DECL`).
7. **Não use o mesmo mecanismo para interface**; trate-o à parte (arquivo 03).
8. **Genéricos**: o .NET compartilha código entre instanciações de referência (`docs/design/coreclr/botr/shared-generics.md`, repo), o que complica o MethodTable (métodos de tipos genéricos vivem no MethodTable canônico). Se sua linguagem tem genéricos reificados, planeje isso cedo.

Pseudocódigo de despacho virtual:

```
call_virtual(obj, slot):
    mt    = obj.header.type          // primeiro campo do objeto
    chunk = mt.indirections[slot / 8]
    code  = chunk[slot % 8]
    return code(obj, args...)
```

## Resumo dos fatos confirmados

- Todo objeto aponta para o seu `MethodTable`; este tem a vtable em chunks de 8 slots (`VTABLE_SLOTS_PER_CHUNK 8`).
- Acesso: `indirections[slot >> 3][slot & 7]` (`GetSlotForVirtual`).
- Chunks podem ser compartilhados entre pai e filho quando o filho não sobrescreve nada neles.
- O comentário da fonte afirma o custo: uma indireção adicional por chamada virtual.
- Não virtuais com slot ficam num array separado, crescendo para trás a partir de `MethodTableAuxiliaryData`.
- `NO_SLOT = 0xffff` é a sentinela de "sem slot".
- Todo `MethodDesc` tem slot, até mesmo abstratos.
- `System.Object` tem slot do finalizador fixado, e o código avisa para não adicionar virtuais à frente.
- VSD não é usado para virtuais de classe (regressão de throughput e startup, segundo o BOTR).

## Etapas de uma chamada virtual (modelo mental)

1. O JIT conhece o número do slot (tempo de compilação) a partir do `MethodDesc` do método declarado.
2. Em runtime, carrega-se o `MethodTable` do objeto (offset 0).
3. Calcula-se o chunk: `slot / 8`; lê-se o ponteiro do chunk em `GetVtableIndirections()`.
4. Lê-se o código no índice `slot % 8`.
5. Chama-se o código com `this` como primeiro argumento.
6. Se o método ainda não foi compilado, o slot aponta para um stub de pré-código que aciona o JIT e depois repatcha o slot.
7. Com tiered compilation, o slot pode ser atualizado novamente quando uma versão otimizada fica pronta.

(As etapas 6-7 descrevem o desenho geral documentado em `method-descriptor.md` e `tiered-compilation.md`; não validei o fluxo exato de backpatching em código.)

## Armadilhas e perguntas de design

- Se a sua vtable é um array simples, mudar o pai exige recompilar os filhos. O mesmo vale aqui.
- Se você compartilha chunks, cuidado com a escrita: patch de slot precisa considerar quem compartilha o chunk (o runtime usa `GetRestoredSlotMT` para saber de onde vem o valor).
- Métodos genéricos e tipos genéricos complicam o conceito de "slot no MethodTable": métodos de instanciações vivem no MethodTable canônico.
- Slots de vtable não são suficientes para chamadas por interface; veja o arquivo 03.
- Tipos de valor não têm vtable própria no valor "solto"; só quando boxed (arquivo 10).
- Arrays têm MethodTable próprio (categoria `Array`), com vtable herdada de `System.Array`.
- Delegates têm métodos fornecidos pelo runtime (categoria EEImpl).

## Referências

- `src/coreclr/vm/methodtable.h`: classe `MethodTable`, seção "The VTABLE" (~1662), `GetSlotForVirtual`, `GetSlotPtrRaw`, `GetNumVirtuals`, `GetNumParentVirtuals`, `GetNumVtableSlots`, `VTABLE_SLOTS_PER_CHUNK`.
- `src/coreclr/vm/methodtable.cpp`, `src/coreclr/vm/methodtable.inl`.
- `src/coreclr/vm/methodtablebuilder.cpp` (~5380-5440 override/newslot; ~4870-4910 MethodImpl).
- `src/coreclr/vm/method.hpp`, `src/coreclr/vm/class.h`.
- `docs/design/coreclr/botr/method-descriptor.md`.
- `docs/design/coreclr/botr/virtual-stub-dispatch.md` (seção "Current State").
- `src/libraries/System.Private.CoreLib/src/System/Object.cs` (comentário sobre slot hardcoded do finalizador).
- `src/coreclr/vm/methoddescbackpatchinfo.h`.
