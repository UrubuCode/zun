# Interface dispatch (interface map, Virtual Call Stub / dispatch stubs)

Convenção: "(repo)" = confirmado em arquivos do repositório; "(inferência)" = dedução minha.

## Mecanismo no .NET

### O problema

Com herança única, um método virtual de classe tem um índice de slot estável em toda a hierarquia. Uma interface não: classes sem relação de herança podem implementá-la, cada uma em posições diferentes. Logo "slot N da vtable" não serve como chave. É preciso mapear (interface, método da interface) -> implementação *no tipo concreto do objeto*.

### Estruturas estáticas por tipo

1. **Interface map**: lista de `InterfaceInfo_t` por `MethodTable`, com `GetNumInterfaces()` e `IterateInterfaceMap()` (`src/coreclr/vm/methodtable.h`) (repo). Inclui interfaces herdadas do pai e as de interfaces base.
2. **Dispatch map**: `class DispatchMap` em `src/coreclr/vm/contractimpl.h` (~linha 873), obtido por `MethodTable::GetDispatchMap()` (`methodtable.h`, ~2542). É um mapa codificado em bytes (`m_rgMap[0]`, com codificação por deltas: `ENCODING_TYPE_DELTA`, `ENCODING_SLOT_DELTA`, `ENCODING_TARGET_SLOT_DELTA`) criado por `DispatchMapBuilder` (~717) (repo).
3. **Tipo no dispatch map é relativo ao tipo implementador**: `DispatchMapTypeID` (contractimpl.h ~77-113) é uma união discriminada entre "esta classe" (`TYPE_ID_THIS_CLASS = 0`), "interface não implementada" e "índice no InterfaceMap" (`inum + 1`) (repo). Assim o mapa não depende de IDs globais.
4. **Quem constrói**: `MethodTableBuilder` (`src/coreclr/vm/methodtablebuilder.cpp/.h`), com `enum class DispatchMapReuseKind` (`methodtablebuilder.h` ~2759) indicando reuso de mapas (por exemplo, de instanciações genéricas: o comentário em `DispatchMap::GetEncodedMapData` diz que a codificação é independente da instanciação) (repo).

O doc oficial diz: "This map is created in BuildMethodTable: it enumerates all interfaces implemented by the type ... and determines every interface method that the type implements or overrides." (`docs/design/coreclr/botr/virtual-stub-dispatch.md`).

### Dispatch token

Token = tupla `<typeID, slot>` do tamanho de uma palavra nativa: `typeID` identifica a interface (ID alocado em runtime, monotonicamente crescente, via um "Type ID Map" em hash) e `slot` é o índice do método na interface no "layout clássico de vtable". Para virtuais de classe o typeID é constante ("this class") (repo, doc de VSD, seções "Design of Token Representation and Dispatch Map" e "Dispatch Tokens"). A classe `DispatchToken` está em `contractimpl.h`.

### Virtual Stub Dispatch (VSD)

Implementação: `src/coreclr/vm/virtualcallstub.h/.cpp`. O call site de interface no código JIT chama através de uma *indirection cell*; o conteúdo da célula é endereço de um stub, que muda com o comportamento observado. O doc descreve três stubs + um resolvedor genérico:

1. **Generic resolver** (função C; `VirtualCallStubManager::ResolveWorker`, ponto de entrada `VSD_ResolveWorker` em `virtualcallstub.h` ~linhas 38 e 509): dado `<token, type>` devolve o alvo; cria stubs, faz backpatch das células, mantém cache e contabilidade.
2. **Lookup stub**: o primeiro atribuído a um call site, criado quando o JIT compila a chamada. Passa token e tipo ao resolvedor; um lookup stub por token único.
3. **Dispatch stub**: para call sites *monomórficos*. Compara o MethodTable do objeto com um tipo em cache; se igual, salta direto ao alvo; senão salta ao resolve stub. Um por `<token, type>`. Em x86, "comparison, conditional failure jump, jump to target".
4. **Resolve stub**: para call sites *polimórficos*. Usa o par `<token, type>` numa cache global; se falhar, chama o resolvedor genérico, que insere a tupla `<token, type, target>` na cache. Um por token, todos compartilham cache global (hash pré-calculado a partir da parte fixa).

Política de promoção/rebaixamento (doc): quando um dispatch stub falha com frequência suficiente, o site é considerado polimórfico e é repatchado para o resolve stub. Em pontos de sincronização (hoje: fim de um GC), sites polimórficos são promovidos aleatoriamente de volta a monomórficos, "under the assumption that the polymorphic attribute of a call site is usually temporary" (repo).

Há também `ResolveWorkerChainLookupAsmStub` (`virtualcallstub.h` ~150) para encadeamento de entradas da cache (repo; detalhes de implementação não aprofundados).

### Estado atual e motivação histórica (doc de VSD)

- Antes: interfaces tinham IDs únicos no processo e todo interface carregado entrava numa tabela global esparsa; em NGEN isso obrigava a "restaurar" todos os tipos que implementam interfaces, aumentando working set de startup. Objetivo do VSD: reduzir working set a frio e distribuir o trabalho ao longo da vida do processo, mantendo throughput equivalente.
- O doc admite que o working set *inicial* por call site é maior (estruturas para rastrear stubs), e que isso prejudicou startup de aplicações cliente.
- VSD só é usado para interfaces; virtuais de classe usam vtable (ver arquivo 02).

Observação de honestidade: o doc é de 2006 (autor Simon Hall) e algumas partes (x86, NGEN) são históricas. Não verifiquei, em `virtualcallstub.cpp`, se há outros mecanismos modernos que alterem o quadro (por exemplo, caminhos para plataformas específicas ou NativeAOT). NativeAOT tem sua própria implementação de dispatch de interface em `src/coreclr/nativeaot/` e `src/coreclr/runtime/`, que não analisei.

### Default interface methods e static abstract

Dispatch de implementação padrão: resolvido no type loader (`MethodTableBuilder::HasDefaultInterfaceImplementation`, ~11725), e em runtime o `ResolveWorker`/mecanismos de resolução precisam considerar a implementação padrão (não li esse trecho). Métodos estáticos virtuais de interface *não* usam VSD: são resolvidos em tempo de JIT/instanciação via `constrained.` (arquivo 05).

## Por quê

1. **Memória/startup**: eliminar a tabela global de IDs de interface e a restauração antecipada (declarado no doc).
2. **Custo proporcional ao uso**: stubs são gerados sob demanda; call sites quentes monomórficos pagam comparação + salto.
3. **Adaptação dinâmica**: o call site pode ser monomórfico, depois polimórfico, depois reavaliado; algo que uma vtable estática não faz (declarado).
4. **Código JIT menor**: o grosso do trabalho está no stub, não no call site (declarado: "results in smaller JITed code").

## Trade-offs

| Tema | Efeito |
|---|---|
| Complexidade | Quatro tipos de componentes (lookup, dispatch, resolve, resolvedor), caches globais, backpatching concorrente |
| Latência da 1ª chamada | Passa pelo resolvedor (lento), depois fica rápido |
| Polimorfismo alto | Cai no resolve stub: hash em cache global; mais lento que vtable |
| Código gerado | Precisa de memória executável e patch seguro; implica W^X e sincronização (inferência; o runtime tem cuidado com isso em `jump-stubs.md` e docs de W^X, não lido aqui) |
| Debug/AOT | Stubs dinâmicos não existem em AOT puro; precisa de alternativa (NativeAOT, não analisado) |
| Virtuais de classe | O próprio doc diz que VSD foi *pior* para eles e foi desabilitado |

## Como aplicar na sua linguagem

Escolha uma estratégia por tamanho de programa e plataforma:

**Nível 1: simples (recomendado para começar).** Cada tipo tem `interfaceMap: Map<InterfaceId, [CodePtr]>`. Chamada: `impl = obj.type.interfaceMap[I][i]`. Custo: lookup em hash por chamada. Melhore com *inline cache* por call site: guarde `(lastType, lastTarget)` no call site e compare.

```
call_iface(obj, I, i, site):
    t = obj.type
    if site.cachedType == t: return site.cachedTarget(obj, ...)
    target = t.dispatchMap[(I, i)]        // lento
    site.cachedType, site.cachedTarget = t, target
    return target(obj, ...)
```

Isto é o análogo do dispatch stub monomórfico do .NET sem geração de código.

**Nível 2: polimórfico.** Adicione cache global `(site/token, type) -> target` (o resolve stub) e promova/rebaixe por contador de falhas.

**Nível 3: tabelas densas (itables).** Se interfaces são poucas e conhecidas em tempo de link, aloque índice por interface e use `type.itable[ifaceIndex][methodIndex]` (o esquema que o .NET abandonou por working set). Vale em linguagens compiladas AOT com programa fechado.

**Nível 4: seletor/hash** (estilo Smalltalk/ObjC) com cache global.

Decisões:

1. IDs de interface devem ser globais ou relativos ao tipo? O .NET usa IDs de runtime para o token, mas o dispatch map é relativo ao implementador (`DispatchMapTypeID`), permitindo reuso de mapas por instanciação genérica.
2. Defina como resolver ambiguidades (duas interfaces com mesmo método; implementação explícita vs implícita; DIM). O `MethodTableBuilder` é onde o .NET faz isso.
3. Planeje o caso de objeto de tipo desconhecido em tempo de compilação com genéricos: se genéricos são reificados e especializados por struct, a chamada de interface pode virar chamada direta (via `constrained.`; arquivo 05).
4. Sincronização: backpatch de call sites é concorrente. Use escrita atômica de um ponteiro e estruturas imutáveis por stub.

## Resumo dos fatos confirmados

- VSD é usado apenas em chamadas de interface (doc BOTR, seção "Current State").
- Token de dispatch = `<typeID, slot>`.
- Existem lookup, dispatch, resolve stubs e um resolvedor genérico.
- Dispatch stub = monomórfico; resolve stub = polimórfico com cache global.
- Sites polimórficos são promovidos aleatoriamente a monomórficos após pontos de sincronização (fim de GC).
- O dispatch map é codificado em bytes (`DispatchMap::m_rgMap`) com deltas, construído por `DispatchMapBuilder`.
- `DispatchMapTypeID` é relativo ao tipo implementador (0 = esta classe; n+1 = índice no interface map).
- `MethodTable::GetDispatchMap()` devolve o mapa; pode não existir.

## Ciclo de vida de um call site de interface

1. O JIT compila a chamada com uma célula de indireção apontando para um lookup stub.
2. Na 1ª chamada, o lookup stub chama o resolvedor com `<token, tipo>`.
3. O resolvedor usa o dispatch map do tipo para achar a implementação.
4. Cria um dispatch stub `<token, tipo>` e repatcha a célula para ele.
5. Chamadas seguintes com o mesmo tipo: comparação + salto.
6. Se o tipo muda com frequência, o dispatch stub falha, e o site passa ao resolve stub.
7. O resolve stub consulta a cache global; em falha, cai no resolvedor, que insere na cache.
8. No fim de um GC, sites polimórficos podem ser devolvidos ao estado monomórfico para reavaliação.

(Resumo do texto do BOTR; a ordem exata de passos internos no código atual não foi auditada.)

## Variantes que sua linguagem pode escolher

| Estratégia | Memória | Latência da chamada quente | Complexidade |
|---|---|---|---|
| Hash por chamada (tipo, método) | baixa | média | baixa |
| Inline cache monomórfico por site | baixa | muito baixa | média |
| Inline cache + cache global | média | baixa | média-alta |
| Itable denso por interface | alta | muito baixa | média |
| Monomorfização total (sem dispatch) | alta (código) | zero | alta (compilador) |

(Tabela qualitativa minha, não medida.)

## Perguntas de design

- Seu programa é aberto (plugins, carga dinâmica) ou fechado (AOT)? Programas fechados podem usar itables densas.
- Você tem JIT ou um interpretador? Stubs com patch exigem código mutável.
- Quanta concorrência há na atualização de call sites? Defina o protocolo (escrita atômica de ponteiro).
- Como você trata interfaces genéricas variantes? O mapa precisa lidar com tipos "equivalentes" por variância.
- Como o debugger e o profiler veem os stubs? O BOTR cita a integração com o code manager.

## Glossário

- Interface map: lista de interfaces implementadas por um `MethodTable`.
- Dispatch map: tabela codificada que liga (interface, método) a slot de implementação.
- Dispatch token: tupla `<typeID, slot>` usada pelos stubs.
- Lookup stub: primeiro stub de um call site; delega ao resolvedor.
- Dispatch stub: stub monomórfico (compara tipo e salta).
- Resolve stub: stub polimórfico com cache global.
- Indirection cell: célula de memória no call site cujo conteúdo é o endereço do stub atual.
- Backpatch: reescrita da célula para apontar para um stub melhor.
- Monomórfico/polimórfico: um call site que vê um único tipo vs. vários tipos.

## Referências

- `docs/design/coreclr/botr/virtual-stub-dispatch.md`: documento principal (tokens, stubs, estado atual).
- `src/coreclr/vm/virtualcallstub.h`, `src/coreclr/vm/virtualcallstub.cpp`: `VirtualCallStubManager`, `VSD_ResolveWorker`, `ResolveWorker`.
- `src/coreclr/vm/contractimpl.h`, `contractimpl.cpp`: `DispatchToken`, `DispatchMapTypeID`, `DispatchMapBuilder`, `DispatchMap`.
- `src/coreclr/vm/methodtable.h`: `GetDispatchMap`, `IterateInterfaceMap`, `ImplementsInterface`, `InterfaceInfo_t`.
- `src/coreclr/vm/methodtablebuilder.h/.cpp`: construção do dispatch map (`DispatchMapReuseKind`).
- `docs/design/coreclr/botr/type-system.md`: menciona VSD como mecanismo para destino de chamadas virtuais de interface.
