# fixed e escopo de pinning

Em C#, `fixed (T* p = &x) { ... }` obtém um ponteiro não gerenciado para memória gerenciada e **impede o GC de mover** o objeto durante o bloco. O escopo "léxico" do `fixed` é, no IL, apenas um **local marcado como `pinned`**. Não existe instrução "pin" nem "unpin": o pinning dura enquanto o local pinned contiver a referência. Este documento detalha o mecanismo, que é pequeno mas cheio de sutilezas.

## Mecanismo no .NET

### 1. `pinned` é modificador do tipo do local

Na assinatura de locals do método, um local pode ter o modificador `ELEMENT_TYPE_PINNED` (`src/coreclr/inc/corhdr.h`, linha 926: `ELEMENT_TYPE_PINNED = 0x05 | ELEMENT_TYPE_MODIFIER`). Em C#, `fixed (int* p = arr)` faz o compilador criar um local `int& pinned` (ou de classe/string) que guarda a referência original; o ponteiro `p` é derivado dele (`conv.u`). Essa tradução é convenção do compilador (fora deste repo); o suporte do runtime é o modificador no tipo do local.

No JIT (`src/coreclr/jit/lclvars.cpp`, `lvaInitTypeRef`, ~linhas 238-257):

```cpp
if ((corInfoTypeWithMod & CORINFO_TYPE_MOD_PINNED) != 0)
{
    if ((corInfoType == CORINFO_TYPE_CLASS) || (corInfoType == CORINFO_TYPE_BYREF))
    {
        JITDUMP("Setting lvPinned for V%02u\n", varNum);
        varDsc->lvPinned = 1;
        if (opts.IsOSR()) { /* OSR may not see refs to the pinned local, but must still report it */
            varDsc->lvImplicitlyReferenced = 1; }
    }
    else { JITDUMP("Ignoring pin for non-GC type V%02u\n", varNum); }
}
```

Duas regras visíveis: (a) só locais cujo tipo é referência a objeto (`CLASS`) ou byref (`BYREF`) são realmente pinned; para outros tipos o pin é **ignorado**; (b) em métodos OSR o local deve ser reportado mesmo sem usos aparentes.

### 2. Como o GC fica sabendo: slot "pinned" na GC info

O JIT emite, para cada slot de pilha pinned, a flag `GC_SLOT_PINNED` (`src/coreclr/inc/gcinfotypes.h`, linha 61: `GC_SLOT_PINNED = 0x2`) em `src/coreclr/jit/gcencode.cpp` (~linhas 4185-4200):

```cpp
// Or in pinned_OFFSET_FLAG for 'pinned' pointer tracking
flags = (GcSlotFlags)(flags | GC_SLOT_PINNED);
```

O GC, ao varrer a pilha do thread, trata slots marcados pinned como raízes que **não podem ser relocadas**; o objeto apontado fica fixo no heap enquanto o slot é reportado. Importante: o local pinned é **sempre não rastreado** ("untracked" / reportado durante todo o método). `src/coreclr/jit/gcinfo.cpp` (~linha 608) afirma: `assert(!varDsc->lvPinned || !varDsc->lvTracked);` ("If is pinned, it must be an untracked local."). Logo, o pinning vale **durante a vida inteira do frame** (do prólogo ao epílogo) e não só enquanto o JIT acha que o local está vivo. O fim do pinning é, portanto, "o método retornar" ou "o local receber outro valor" (ver abaixo).

### 3. Atribuir ao local pinned = desfazer o pin

Como o pin é uma propriedade do *valor* contido no slot, escrever `null` (ou outra referência) no local libera o objeto anterior. O compilador C# usa isso para encerrar o `fixed`: ao sair do bloco, zera o local (convenção). O JIT trata isso com cuidado em `importer.cpp` (~linha 7087):

```cpp
// Stores to pinned locals can have the implicit side effect of "unpinning", so we must spill
// things that could depend on the pin.
if (lvaTable[lclNum].lvPinned) { impSpillSideEffects(false, CHECK_SPILL_ALL DEBUGARG("Spill before store to pinned local")); }
```

O TODO no comentário reconhece que a dependência pode ser de qualquer coisa. Ou seja, um *store* num local pinned é uma barreira de ordenação para código que usa ponteiros derivados. Em `forwardsub.cpp` (~linha 617) o JIT também evita fazer forward substitution de locais pinned ("Leave pinned locals alone").

### 4. Inlining

Quando um método com `fixed` é inlinado, os locais do inlinee preservam `pinned`: `importer.cpp` (~linha 14513) faz `lvaTable[tmpNum].lvPinned = inlineeLocal.lclIsPinned;`. Isso mantém a semântica ao compor.

### 5. Pinning em runtime-async

A especificação de runtime-async (`docs/design/specs/runtime-async.md`) determina: "pinning locals may not be 'hoisted' across suspension points and will have `null` value after a suspension point." Ou seja, **pin não sobrevive a `await`**: após a retomada, o local pinned vale `null`. Isso é coerente: a continuação é um objeto no heap, e manter um pin em algo que o framework move seria impossível sem handle pinned. As regras de linguagem sobre `fixed` combinado com `await` pertencem ao compilador e não são verificáveis aqui.

### 6. Formas alternativas de pinning no runtime

- **`GCHandleType.Pinned`** (`src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/GCHandleType.cs`: `Weak = 0, WeakTrackResurrection = 1, Normal = 2, Pinned = 3`) e `GCHandle.Alloc(obj, GCHandleType.Pinned)` (`GCHandle.cs`, linhas ~37-50; recusa objetos não "pinnable": `Marshal.IsPinnable`). O pin vive até `Free()`, independente de frames.
- **`GC.AllocateArray<T>(int length, bool pinned = false)`** (`src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs`, linha ~892; também no NativeAOT): arrays alocados já na região de objetos fixos (Pinned Object Heap), sem precisar de `fixed` nem handle.
- **`GetPinnableReference()`**: padrão pelo qual um tipo diz ao `fixed` o que pinnar. Por exemplo `String.GetPinnableReference() => ref _firstChar` (`String.cs`, linha ~529) e `Span<T>.GetPinnableReference()` (`Span.cs`, linha 283, "It can be used for pinning and is required to support the use of span within a fixed statement"; para span vazio retorna referência nula). `[EditorBrowsable(Never)]` indica que é um padrão para o compilador, não API pública de uso normal.
- **Blittable/unmanaged em pilha**: `stackalloc` não precisa de pin (não é movido).

### 7. Pinning e funclets/filtros

`docs/design/coreclr/botr/clr-abi.md` (linha ~444) mostra uma terceira utilização do mesmo mecanismo: no filtro (1ª passada), o JIT codifica "all stack slots that are live within the filter as being pinned" para evitar relocação dupla quando um slot é reportado duas vezes (frame do filtro e frame do pai). Nesse caso o pin é usado como técnica interna do JIT/GC, e não por `fixed`.

Também, no alvo WebAssembly, "refs homed on the linear stack are reported pinned" (`gcencode.cpp` ~linha 4190 e `clr-abi.md` ~linha 892), pelo fato de a pilha de operandos da Wasm ser invisível ao GC.

### 8. Custo de pinning

O pin impede compactação em volta do objeto, fragmentando o heap. Isso não está medido neste repositório, mas é a motivação conhecida para `GC.AllocateArray(pinned: true)` (POH) e para recomendar pins de curta duração. A documentação do GC (`docs/design/coreclr/botr/garbage-collection.md`) descreve os tipos de heap e segmentos, incluindo o heap de objetos fixos.

## Por quê

1. **Mínimo de mecanismo.** Um modificador de tipo de local + um bit na GC info bastam; não precisa de instruções novas nem de tabelas de regiões.
2. **Custo quase nulo.** Sem alocação de handle; o pin é só um slot de pilha que o GC reconhece.
3. **Escopo por frame ou por atribuição.** O tempo de vida do pin é o do slot, simples de raciocinar e de implementar no GC (varredura de pilha já existente).
4. **Interoperabilidade com código nativo.** P/Invoke e APIs unsafe precisam de ponteiros estáveis durante a chamada.
5. **Segurança do GC.** Restringir `pinned` a `CLASS`/`BYREF` evita que o GC tenha de interpretar slots arbitrários.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Pin = tipo do local (frame inteiro) | Sem custo por entrada/saída | Pin dura mais que o necessário se o compilador não zera o local; local pinned é "untracked" (reportado o tempo todo) |
| Fim do pin por atribuição | Sem instrução `unpin` | Stores em locais pinned são barreiras para o JIT (spill, sem forward substitution) |
| Pin ignorado para tipos não GC | Sem erro | Pode confundir; `fixed` em tipo sem ref não faz nada |
| Pin não atravessa suspensão async | Modelo simples de continuação | Código com `await` precisa de `GCHandle` ou POH para pins longos |
| `GCHandle.Pinned` | Pin entre chamadas e threads | Custo de handle; vazamento se esquecer `Free` |
| `AllocateArray(pinned: true)` | Sem fragmentação do heap normal | Só para arrays; API específica |
| `GetPinnableReference` padrão | Extensível a tipos próprios | Convenção de nome; fácil de implementar errado |
| Pins fragmentam o heap | Funciona com GC compactante | Pressão de fragmentação em cargas com muitos pins longos |

## Como aplicar na sua linguagem

1. **Decida se precisa de pinning.** Se seu GC for não-movente (mark-sweep sem compactação) ou se sua linguagem não expõe ponteiros para o heap, pode não precisar. Se precisa de interop nativo com GC movente, projete o pin desde o início.
2. **Escolha o modelo de escopo**:
   - *tipo do local* (como o .NET): o pin dura o frame ou até reatribuição;
   - *instruções* `pin`/`unpin` explícitas ou região (`pin_begin`/`pin_end`): escopo exato, mas precisa de tabela/estado extra;
   - *handles* (`GCHandle`): independente de frame, com custo e risco de vazamento.
   O .NET oferece os três, para necessidades diferentes.
3. **Defina claramente o fim do pin**. Se for por atribuição, faça o compilador zerar o local no fim do bloco `fixed` e documente que o JIT/otimizador trata a atribuição como barreira.
4. **Restrinja o que pode ser pinned** (tipos de referência e byrefs/refs interiores), como `lvPinned` faz.
5. **Ofereça um protocolo `GetPinnableReference`** (ou equivalente) para que tipos próprios (strings, spans, buffers) definam *qual* referência fixar, inclusive o caso vazio.
6. **Proíba pin através de pontos de suspensão** (`await`/`yield`) ou defina claramente que o valor é perdido (como o runtime-async). Mais seguro: erro de compilação.
7. **Considere uma região para objetos longamente fixos** (POH): evita fragmentação do heap principal e é melhor que pins curtos repetidos em hot paths de I/O.
8. **Informe o GC**: o formato de GC info deve poder marcar slots como pinned/interior; o coletor deve respeitar isso na fase de relocação.
9. **Documente a interação com exceções/filtros**: se seu runtime executa filtros antes do desenrolar, você pode precisar de pins internos para dupla-reportagem (como no ABI do .NET).
10. **Compile com cuidado em otimizações**: forward substitution, hoisting e eliminação de stores em locais pinned podem estender ou encurtar o pin.

## Apêndice: comparação das formas de pinning

| Forma | Duração | Custo | Onde está no repo | Observação |
|---|---|---|---|---|
| Local `pinned` (`fixed`) | Até o fim do frame ou até reatribuir o local | Um slot de pilha reportado ao GC como `GC_SLOT_PINNED` | `lclvars.cpp`, `gcencode.cpp` | Não atravessa suspensão async (vira `null`) |
| `GCHandle` tipo `Pinned` | Até `Free()` | Alocação de handle na tabela de handles | `GCHandle.cs`, `GCHandleType.cs` | Recusa objetos não pinnable (`Marshal.IsPinnable`) |
| Array no POH | Vida do objeto | Alocado em região própria; sem fragmentar | `GC.AllocateArray<T>(length, pinned: true)` | Só arrays |
| Slots pinned internos (filtros) | Duração do filtro | Reporte duplicado evitado | `clr-abi.md` ~444 | Mecanismo do JIT, não exposto a usuário |
| Slots na pilha linear (Wasm) | Chamada | Reporte como pinned | `gcencode.cpp` ~4190 | Específico do alvo Wasm |

## Apêndice: sequência de eventos de um `fixed` (conceitual)

1. O compilador declara um local com tipo `T& pinned` (ou classe `pinned`) na assinatura de locals do método.
2. O JIT, ao inicializar locais (`lvaInitTypeRef`), marca `lvPinned = 1`.
3. O código atribui ao local a referência original (por exemplo, `ref arr[0]`); a partir daí o slot contém uma raiz pinned.
4. Ponteiros derivados (`conv.u` do byref) são usados no corpo. O objeto não se move enquanto o slot o referencia.
5. Ao sair do bloco, o compilador atribui `null` ao local (convenção). O JIT trata o store como barreira (`impSpillSideEffects`).
6. Em caso de exceção, o slot continua reportado até o frame ser desenrolado; o objeto é liberado do pin quando o frame deixa de existir.
7. Após o método retornar, o slot desaparece com o frame.

Consequência para o design: **o ponteiro derivado é só um inteiro**. Se o objeto for movido por falha do pin (local zerado cedo), o ponteiro fica solto; é por isso que a linguagem só permite o ponteiro dentro do bloco.

## Apêndice: o que um verificador/JIT da sua linguagem deveria impor

- Pin só em referências GC (ignorar ou rejeitar outros tipos, como em `lclvars.cpp`).
- Locais pinned são sempre reportados (não rastreados): pagam custo de GC info por frame inteiro. Limite a quantidade se o método for muito quente.
- Nenhum otimizador pode mover, duplicar ou propagar o valor de um local pinned sem manter a vida do pin (`forwardsub.cpp` evita).
- Nenhum pin deve atravessar suspensão de corrotina, a menos que a continuação tenha um mecanismo próprio (ex.: handle).
- Em transformações como inlining, a propriedade `pinned` deve ser herdada pelos locais do inlinee (`importer.cpp` ~14513).

## Referências

- `src/coreclr/inc/corhdr.h` (linha 926: `ELEMENT_TYPE_PINNED`).
- `src/coreclr/jit/lclvars.cpp` (~238-257: `lvPinned`, OSR; ~2404, ~3314, ~3789: usos adicionais).
- `src/coreclr/jit/gcencode.cpp` (~4185-4200, ~2268: `GC_SLOT_PINNED`), `src/coreclr/jit/gcinfo.cpp` (~608).
- `src/coreclr/inc/gcinfotypes.h` (linha 61: `GC_SLOT_PINNED = 0x2`).
- `src/coreclr/jit/importer.cpp` (~7087: store em local pinned; ~14513: inlining), `src/coreclr/jit/forwardsub.cpp` (~617).
- `docs/design/specs/runtime-async.md` (pinning não atravessa suspensão), `docs/design/coreclr/botr/runtime-async-codegen.md`.
- `docs/design/coreclr/botr/clr-abi.md` (~444: filtros e slots pinned; ~892: Wasm), `docs/design/coreclr/botr/garbage-collection.md`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/GCHandle.cs`, `GCHandleType.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` (`AllocateArray<T>(int, bool pinned)`).
- `src/libraries/System.Private.CoreLib/src/System/String.cs` (~529), `Span.cs` (~283): `GetPinnableReference`.

> Nota de confiança: a tradução exata de `fixed` pelo compilador C# (local `pinned`, `conv.u`, zerar ao sair) é convenção do compilador; o suporte do runtime e do JIT ao modificador `pinned` está confirmado nos arquivos citados.
