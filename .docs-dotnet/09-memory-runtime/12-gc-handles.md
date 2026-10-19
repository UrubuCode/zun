# GC handles (weak, pinned, normal, dependent)

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.
> Código-fonte do GC: `src/coreclr/gc/gcinterface.h` (tipos), `handletable*.cpp/h`,
> `objecthandle.cpp/h`, `gchandletable.cpp`, `gchandletableimpl.h`.
> Lado gerenciado: `System/Runtime/InteropServices/GCHandle*.cs`,
> `src/coreclr/System.Private.CoreLib/src/System/Runtime/DependentHandle.cs`.

## Mecanismo no .NET

### O que é um GC handle

Um **handle** é uma entrada numa **tabela de handles** mantida pelo GC, que guarda uma referência
a um objeto gerenciado e define *como* o GC deve tratá-la. O código gerenciado/nativo recebe
um valor opaco (um `IntPtr` / `OBJECTHANDLE`) em vez do ponteiro cru. Isso resolve dois
problemas: (1) referências a objetos gerenciados vindas de fora do heap (memória nativa,
código de interop) precisam ser **raízes** do GC; (2) como objetos **se movem**, quem está
fora não pode guardar o endereço - o GC atualiza o conteúdo do handle.

`GCHandle` (`GCHandle.cs`): "Representa um GC handle opaco para um objeto gerenciado. Um GC
handle é usado quando uma referência a objeto precisa ser alcançável a partir de memória não
gerenciada." O campo é só `private IntPtr _handle;`.

### Os tipos de handle (o enum do GC)

`gcinterface.h` (comentários de cada valor):

| Tipo | Valor | Semântica (comentário do código) |
|---|---|---|
| `HNDTYPE_WEAK_SHORT` | 0 | Fraco de vida curta: rastreia o objeto até ser detectado inalcançável **pela primeira vez**; é cortado mesmo que o objeto vá aparecer no grafo de finalização pendente; não acompanha ressurreições |
| `HNDTYPE_WEAK_LONG` (= `WEAK_DEFAULT`) | 1 | Fraco de vida longa: rastreia até o objeto ser **de fato recolhido**; continua através da finalização e de ressurreições |
| `HNDTYPE_STRONG` (= `DEFAULT`) | 2 | Forte: age como referência normal; o objeto é promovido (permanece vivo) |
| `HNDTYPE_PINNED` | 3 | Forte + **impede que o objeto se mova** durante o GC. Comentário em maiúsculas: caro, usar com parcimônia |
| `HNDTYPE_VARIABLE` | 4 | Tipo mutável em tempo de execução. "Not used currently" |
| `HNDTYPE_REFCOUNTED` | 5 | Forte enquanto o contador de referência > 0; fraco caso contrário |
| `HNDTYPE_DEPENDENT` | 6 | Par primário/secundário: o secundário vive enquanto o primário viver, sem manter o primário vivo |
| `HNDTYPE_ASYNCPINNED` | 7 | Pin de buffers de `Overlapped`. "No longer used in the VM starting .NET 8" (mantido por compatibilidade) |
| `HNDTYPE_SIZEDREF` | 8 | Forte com dado de tamanho associado. "No longer used starting .NET 9" |
| `HNDTYPE_WEAK_NATIVE_COM` | 9 | Fraco para RCW de COM. "No longer used starting .NET 8" |

(O enum tem mais valores depois desses; lidos apenas até o 9.) Vários comentários trazem a
marca "[cDAC] [GC]: Contract depends on this value" - o valor numérico é parte de um contrato
com o diagnóstico (cDAC), não pode ser alterado livremente.

No lado gerenciado `GCHandleType` (`GCHandleType.cs`) expõe só quatro: `Weak = 0`,
`WeakTrackResurrection = 1`, `Normal = 2`, `Pinned = 3`, e o construtor de `GCHandle` valida
`(uint)type > (uint)GCHandleType.Pinned` ("IMPORTANT: This must be kept in sync with the
GCHandleType enum"). Documentação em `GCHandle.cs`:

- **Normal**: impede o objeto de ser coletado.
- **Weak**: permite coleta; o conteúdo do handle é zerado. "Weak references are zeroed before
  the finalizer runs, so if the object is resurrected in the finalizer the weak reference is
  still zeroed."
- **WeakTrackResurrection**: como Weak, mas só zera depois de o objeto realmente sumir.
- **Pinned**: como Normal, mas permite tomar o endereço do objeto.

Para `Pinned`, o construtor valida `Marshal.IsPinnable(value)` e faz `handle |= 1` para
**marcar o bit de "fixado"** no valor do handle (guardado no `IntPtr`). Também existem tipos
genéricos tipados: `GCHandle<T>`, `PinnedGCHandle<T>`, `WeakGCHandle<T>`
(`GCHandle.T.cs`, `PinnedGCHandle.T.cs`, `WeakGCHandle.T.cs`).

### Dependent handle em detalhe

`DependentHandle` (`DependentHandle.cs`) define: "um dependent handle conditionally mantém viva
uma instância *dependente* **enquanto** a instância *alvo (target)* estiver viva, sem ser uma
referência forte para o alvo". Conceitualmente: "uma referência fraca ao alvo A, com A tendo um
campo forte para B". Não é thread-safe quanto a `Dispose`; "você deve chamar `Dispose()` ou
causará um vazamento na handle table nativa".

Uso real: `ConditionalWeakTable<TKey,TValue>` guarda `DependentHandle depHnd` por entrada
(`ConditionalWeakTable.cs:546,618`: "Holds key and value using a weak reference for the key and
a strong reference for the value"). O comentário em `gcinterface.h` também diz que são usados
em **Edit-and-Continue** para adicionar campos a objetos existentes.

Na implementação (`objecthandle.cpp:823`): o primário fica no handle e o secundário em
"extra info" (`HndSetHandleExtraInfo(handle, HNDTYPE_DEPENDENT, ...)`). A promoção exige um
**ponto fixo**: `Ref_ScanDependentHandlesForPromotion` (linha 1167) varre a tabela promovendo o
secundário cujo primário foi promovido; comentário: "Multiple scans may be required since (a)
secondary promotions made during one scan could cause the primary of another handle to be
promoted and (b) the GC may not have marked all promoted objects at the time it initially calls
us." Retorna `true` se houve alguma promoção, para iterar de novo. Há também
`Ref_ScanDependentHandlesForClearing` (1236) e `...ForRelocation` (1304).

### Handles dentro do ciclo de GC (`objecthandle.cpp`)

As funções `Ref_*` são os pontos de integração com as fases do GC:

| Função | Papel |
|---|---|
| `Ref_TraceNormalRoots` (981) | Raízes de handles fortes durante o mark |
| `Ref_TracePinningRoots` (930) | Raízes de handles fixados (pins) |
| `Ref_TraceRefCountHandles` (1056) | Refcounted |
| `Ref_ScanDependentHandlesForPromotion` (1167) | Promoção iterativa de dependentes |
| `Ref_CheckReachable` (1086) / `Ref_CheckAlive` (1522) | Verifica alcançabilidade para os fracos (decidir quem zerar) |
| `Ref_ScanDependentHandlesForClearing` (1236) | Zera dependentes cujo primário morreu |
| `Ref_UpdatePointers` (1570) | **Relocação**: atualiza conteúdo dos handles após mover |
| `Ref_ScanDependentHandlesForRelocation` (1304) | Relocação de dependentes |
| `Ref_AgeHandles` (1773) | Envelhece handles junto com as gerações |

### A handle table também é "geracional"

`handletablepriv.h` (linhas ~36-48): o cabeçalho de cada segmento da tabela tem
`rgGeneration`, descrito como **"Write Barrier Generation Numbers"**: "Cada byte corresponde
a um *clump* de handles. O valor do byte é a menor geração possível para a qual um handle
daquele clump pode apontar." Ou seja, a tabela de handles tem o seu próprio mecanismo
análogo a cards (arquivo 03): numa coleta efêmera o GC só varre clumps que podem conter
referências para a geração condenada. Para isso `Ref_AgeHandles` ajusta esses números.
Outros campos do segmento: `rgAllocation` (cadeia de blocos), máscaras de livres, `rgBlockType`,
`rgUserData`, `rgLocks`, `rgTail`, `rgHint`, `rgFreeCount`; além de um `rgQuickCache` por tipo
(operações "interlocked") para alocação rápida. **[confirmado]** nos nomes de campo; a mecânica
detalhada do envelhecimento não foi lida.

### Interfaces, lojas (stores) e GC substituível

`gchandletableimpl.h` mostra `GCHandleStore : IGCHandleStore` (`CreateHandleOfType`,
`CreateHandleWithExtraInfo`, `CreateDependentHandle`, `Uproot`) e
`GCHandleManager : IGCHandleManager` (`CreateGlobalHandleOfType`, `DestroyHandleOfType`,
`StoreObjectInHandle`, `SetExtraInfoForHandle`, ...). O runtime acessa a handle table **por
interface** (`gcinterface.h`), de modo que um GC independente pode trazer sua própria
implementação (`docs/design/features/standalone-gc-loading.md`). Uma `HandleStore` por
domínio/contexto permite descartar tudo de uma vez (`Uproot`, `DestroyHandleStore`).

### Alocação de handle a partir do código gerenciado

`GCHandle.CoreCLR.cs`: `InternalAlloc(value, type)` chama um `_InternalAlloc` (internal call
rápido) e, se não houver espaço sem transição, recai em `InternalAllocWithGCTransition`
(QCall, `GCHandle_InternalAllocWithGCTransition`). **[confirmado]** pelos nomes; o motivo
exato da duplicidade é **[inferido]**: alocação rápida sem poder disparar GC vs. alocação que
pode precisar de transição para modo cooperativo/preemptivo.

### Referências fracas de alto nível

`WeakReference` / `WeakReference<T>` (`System/WeakReference*.cs`) usam handles fracos por baixo
(`WeakReferenceHandleTags` marca bits do valor, incluindo um bit para
`ComAwareWeakReference`). `WeakReference` é uma classe com finalizador: o comentário em
`WeakReference.cs` alerta para ataques de finalização ("if the WeakReference instance is
finalized away underneath you...").

## Por quê

- **Raízes explícitas** para ponteiros que vivem fora do heap gerenciado - necessário para
  interop, tabelas globais e async nativo.
- **Fraco curto vs. longo** existe por causa da **finalização**: o curto é limpo antes da
  ressurreição (seguro para caches que não devem ver objetos "em finalização"); o longo
  acompanha o objeto até o fim, para quem precisa observar objetos ressuscitados.
- **Pinned como tipo de handle**: o GC precisa saber que o objeto não pode ser movido (arquivo
  10); manter isso na tabela de handles dá ao plan phase uma lista direta de pins
  (`Ref_TracePinningRoots`).
- **Dependent**: implementa corretamente *ephemerons* (mapa fraco por chave) sem criar ciclos
  que vazam: sem ele, `ConditionalWeakTable` com valor referenciando a chave nunca seria
  coletado. A necessidade de ponto fixo vem da dependência cruzada entre handles.
- **Handle table geracional**: evita varrer todos os handles em cada GC de gen0.
- **Interface abstrata**: para permitir GC plugável e testar variantes.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Handle opaco (indireção) | GC pode mover o objeto; seguro com código nativo | Uma indireção por acesso; precisa de `Free` manual (vazamento se esquecer) |
| Tabela de handles separada do heap | Varredura rápida das raízes | Estrutura extra com seu próprio write barrier/geração |
| Weak short + long | Semântica correta com finalização | Dois conceitos para o usuário entender |
| Pinned handle | Interop sem cópia | Fragmentação (arquivo 10) |
| Dependent handle | Ephemeron correto | Promoção iterativa (ponto fixo) no mark: custo extra e complexidade |
| Valores numéricos de tipos fixados (cDAC) | Ferramentas de diagnóstico estáveis | Não se pode reordenar o enum; tipos mortos permanecem |
| Tipos genéricos (`GCHandle<T>`) | Menos casts; segurança de tipos | Mais API |
| Interface `IGCHandleManager` | GC substituível | Superfície de API a manter |

## Como aplicar na sua linguagem

1. **Ofereça pelo menos 3 handles**: forte, fraco e fixado (pinned). Sem forte, não há como
   passar objeto para código nativo; sem fraco, caches e eventos vazam; sem pin, não há
   zero-cópia (se o GC move).
2. **Decida a semântica do fraco em relação à finalização**: se você tem finalizadores
   (arquivo 11), precisa dos dois níveis (curto/longo) ou deve proibir ressurreição.
   Sem finalizadores, um nível basta.
3. **Se quiser mapas fracos corretos (ephemerons), implemente o handle "dependent"**:
   estrutura `(primário, secundário)` e um loop de ponto fixo no mark. Use-o para construir
   `ConditionalWeakTable`/`WeakMap`.
4. **Faça a tabela de handles geracional**: guarde por *clump* a menor geração apontada e
   atualize junto ao write barrier/aging, para coletas efêmeras não varrerem tudo.
5. **Sirva os handles por interface** (`CreateHandleOfType`, `DestroyHandleOfType`,
   `StoreObjectInHandle`...). Isso isola a política de GC.
6. **Torne `Free` obrigatório e rastreável**: o próprio doc de `DependentHandle` avisa sobre
   vazamento na handle table nativa. Considere finalizadores/`SafeHandle`-like para o
   próprio handle, e eventos que contem handles por tipo.
7. **Reserve tipos para evolução**: o enum do .NET tem tipos "não mais usados" que continuam
   ocupando valores por compatibilidade. Se for expor o enum publicamente, planeje o
   versionamento desde o começo.
8. **Cuide de threads**: operações de handle (`Target`, `Dependent`) devem ser thread-safe
   exceto `Dispose`; deixe isso documentado.
9. **API de alto nível**: exponha `WeakReference` e `ConditionalWeakTable` e esconda `GCHandle`
   atrás deles; só uma minoria precisa dos handles crus.
10. **Teste com GC estressado**: handles errados causam falhas esporádicas (objeto
    coletado enquanto nativo usa; ou movido enquanto se usa o endereço).

## Referências

- `src/coreclr/gc/gcinterface.h` - enum `HNDTYPE_*` e comentários (linhas ~405-540);
  `IGCHandleStore`/`IGCHandleManager`.
- `src/coreclr/gc/objecthandle.cpp` - `Ref_TraceNormalRoots`, `Ref_TracePinningRoots`,
  `Ref_ScanDependentHandlesForPromotion/Clearing/Relocation`, `Ref_CheckAlive`,
  `Ref_UpdatePointers`, `Ref_AgeHandles`; `HndSetHandleExtraInfo` (823).
- `src/coreclr/gc/handletable.cpp`, `handletablecore.cpp`, `handletablescan.cpp`,
  `handletablecache.cpp`, `handletablepriv.h` (segmento: `rgGeneration`...), `handletable.inl`,
  `handletableconstants.h`.
- `src/coreclr/gc/gchandletable.cpp`, `gchandletableimpl.h` - implementação das interfaces.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/GCHandle.cs`,
  `GCHandleType.cs`, `GCHandle.T.cs`, `PinnedGCHandle.T.cs`, `WeakGCHandle.T.cs`,
  `GCHandleExtensions.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/DependentHandle.cs`,
  `.../InteropServices/GCHandle.CoreCLR.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ConditionalWeakTable.cs`.
- `src/libraries/System.Private.CoreLib/src/System/WeakReference.cs`, `WeakReference.T.cs`.
- `docs/design/features/standalone-gc-loading.md`.
