# Decisões de alocação: pilha vs. heap (escape analysis)

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.
> Código principal: `src/coreclr/jit/objectalloc.cpp` / `objectalloc.h`.
> Documento de design: `docs/design/coreclr/jit/object-stack-allocation.md`.

## Mecanismo no .NET

### Duas camadas de decisão

1. **Na linguagem/tipo**: tipos *valor* (`struct`) vivem inline (pilha, campo, registrador);
   tipos *referência* (`class`) são alocados no heap gerenciado por padrão. Isso é decisão
   do **programador**. O CLR também permite memória na pilha explícita via `stackalloc`
   (`localloc` na IL), normalmente exposta como `Span<T>` (arquivo 08).
2. **No JIT**: a fase `Allocate Objects` (`PHASE_ALLOCATE_OBJECTS`, `compphases.h`) executa
   **escape analysis** e **move alocações de objetos/arrays para a pilha** quando prova que
   o objeto não "escapa" do método. Isso é decisão **automática do compilador**.

### Definição de escape (design doc)

"Um objeto escapa de um método se pode ser acessado depois que a execução do método
terminou. Uma alocação só pode ser movida para a pilha com segurança se o objeto não escapa
do método alocador." (`object-stack-allocation.md`). Benefícios listados: menos pressão
no GC (heap menor, sem GC para alocar/liberar), campos podem ser promovidos a registradores
(scalar replacement), e algumas inicializações com zero podem ser removidas.

### Como o JIT implementa

Em `objectalloc.cpp` (construtor e `DoPhase`, linhas ~35-256):

- A fase só roda se o método tem `newobj` ou `newarr` (`OMF_HAS_NEWOBJ`/`OMF_HAS_NEWARRAY`).
- Constrói um **grafo de conexões** ("connection graph"): nós são, em sua maioria, locais
  de tipo GC, e arestas mostram como um local pode atribuir valores a outro. Há nós
  abstratos: uma "fonte desconhecida de valores", pseudo-nós para atribuições condicionais, e
  nós para campos de structs locais.
- `MarkEscapingVarsAndBuildConnGraph` percorre o IR; `AnalyzeParentStack` examina o contexto
  de uso (pais na árvore) para ver se o uso é "trivial" (não escapa) ou se o ponteiro vaza
  (passado a chamada, armazenado em heap, retornado etc.).
- `ComputeEscapingNodes` + `ComputeConnGraphClosure` calculam o **fecho transitivo**: se um nó
  escapa, tudo que ele pode alimentar também escapa.
- `MorphAllocObjNodes` troca a alocação por stack allocation quando `CanAllocateLclVarOnStack`
  é verdadeiro; o JIT registra o **motivo** quando não pode (`m_onHeapReason`, ex.:
  `"[too large]"`, `"[alloc in loop]"`, `"[disabled by config]"`).
- Depois, `ComputeStackObjectPointers` e `RewriteUses` ajustam tipos (de `TYP_REF` para
  `TYP_BYREF` etc., via `UpdateStoreType`/`UpdateAncestorTypes`), pois referências para objetos
  na pilha precisam ser tratadas como ponteiros interiores/não-heap.

### Condições que impedem alocação em pilha (lidas no código)

| Condição | Onde |
|---|---|
| O objeto escapa (`CanLclVarEscape`) | `CanAllocateLclVarOnStack` |
| Sem uso não-trivial do local (`!IsLclVarUsed`) - fica no heap e o DCE limpa | idem |
| Tamanho > `m_StackAllocMaxSize` (config `JitObjectStackAllocationSize`, padrão **528**) | idem, `"[too large]"` |
| Array com comprimento negativo/ > `CORINFO_Array_MaxLength`, ou grande demais (`IsArrayTooLarge`) | idem |
| Alocação em bloco com **salto para trás** (possível loop): `BBF_BACKWARD_JUMP` | `MorphAllocObjNodeHelper`, `"[alloc in loop]"` |
| Tipo **com finalizador** ou tipo COM | `CEEInfo::canAllocateOnStack` (`src/coreclr/vm/jitinterface.cpp:1877`): `!pMT->HasFinalizer()` e `!IsComObjectType()` |
| Configurações desligadas (`JitObjectStackAllocation*`) | `jitconfigvalues.h:722-732` |

Configurações (todas `RELEASE_CONFIG_INTEGER`, padrão 1 salvo indicado):
`JitObjectStackAllocation`, `...RefClass`, `...BoxedValueClass`, `...ConditionalEscape`,
`...Array`, `...Size` (528), `...TrackFields`. Ou seja, a otimização está **ligada por padrão**
neste código-fonte, para classes, valores boxed e arrays de tamanho constante.

### Escape condicional e enumeradores

O código tem "conditional escape analysis" ligada a **enumeradores**: quando o JIT, via
*guarded devirtualization* (arquivo 07), vê um `IEnumerable<T>` provavelmente de um tipo
concreto, o `GetEnumerator()` que criaria um objeto pode ser tornado não-escapante em um
dos caminhos. O código mantém `m_EnumeratorLocalToPseudoIndexMap`, pseudo-nós e "clonagem"
(`CloneAndSpecialize`, `CheckForEnumeratorUse`, `AnalyzeIfCloningCanPreventEscape`): clona
a região de código para garantir que no clone o objeto não escape. Doc relacionado:
`docs/design/coreclr/jit/DeabstractionAndConditionalEscapeAnalysis.md`.
Fatos **[confirmados]** pelos nomes e fluxo; detalhes finos não foram verificados.

### Interação com o GC e o write barrier

Do design doc (seção "GC considerations"):

- Referência que sempre aponta para objeto no heap: reportada como `TYPE_GC_REF`.
- Referência que sempre aponta para objeto na pilha: **não reportada** ao GC.
- Referência que pode ser heap ou pilha: `TYPE_GC_BYREF`.
- Todos os **campos GC** de objetos na pilha precisam ser reportados ao GC (como em structs).
- Ao modificar um campo: destino sempre heap => write barrier normal; sempre pilha => nenhum
  barrier; incerto => barrier **checked** (arquivo 03).

### Limites históricos e abordagem em camadas

O doc discute onde a análise deveria rodar: no JIT (vê o callee, mas processa os métodos
"de cima para baixo", o que combina mal com stack allocation); em crossgen/R2R (mais tempo,
mas sem permissão de olhar código de outros assemblies e com restrições de versionamento
porque `newobj` dependeria do layout de classes); no ILLink (visão do programa inteiro em
apps self-contained). A decisão adotada na implementação atual é a **análise no JIT**, com
foco em casos que não exigem análise interprocedural profunda, apoiada por **inlining**
(arquivo 07). Há um campo `m_isR2R` no construtor, indicando tratamento diferente em R2R.

Experimentos do doc (2016, protótipo): com a suposição otimista de que argumentos não
escapam, "424 de 6150 métodos tiveram alocações movidas, 586 de 7977 sites"; excluindo
exceções (5345 sites eram alocações de exceções), 586/2632 (**22,2%** como limite
superior). Esses números são do protótipo histórico, não da implementação atual.

### Outras formas de "não alocar no heap"

- **Heap congelado / NonGC Heap** (`docs/design/features/NonGC-Heap.md`): objetos imortais
  (ex.: literais string) ficam fora do GC; o JIT pode embutir a referência diretamente no
  código. É uma decisão de alocação "para fora do heap coletável".
- **Pinned Object Heap** (arquivo 10) - decisão de alocação por finalidade.
- **Large Object Heap** - por tamanho (85 000 bytes, arquivo 01).
- **`localloc` fixo pequeno vira local**: `DEFAULT_MAX_LOCALLOC_TO_LOCAL_SIZE = 32`
  (`compiler.h`).
- **Structs / box elimination**: com `JitObjectStackAllocationBoxedValueClass`, o JIT pode
  alocar na pilha o *box* de um valor.

## Por quê

- GC tem custo por objeto alocado (e por sobrevivência); alocação em pilha é "de graça" e
  some quando o método retorna.
- Em .NET, pelos próprios autores, a otimização "é mais importante em Java, que não tem value
  types" (doc). Como .NET tem `struct`, parte do ganho já está nas mãos do programador;
  a análise recupera os casos de abstração (delegates temporários, enumeradores,
  boxing, arrays pequenos) - ver também `Deabstraction...md`.
- Escape analysis **no JIT** funciona para qualquer linguagem que gere IL, sem mudança no
  compilador de origem (ponto a favor listado no doc: "will apply to any msil code regardless of the language compiler").
- Limites (528 bytes, nada em loops, nada com finalizador) mantêm a pilha previsível e
  evitam crescimento sem limite do frame **[inferido]** (o doc lista "limite de tamanho" e
  alocações em loops como restrições).

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Escape analysis intra-procedural + inlining | Barato, adequado a JIT | Chamadas não inlinadas fazem os argumentos "escaparem"; muitas oportunidades perdidas |
| Análise interprocedural | Mais precisa | Cara no JIT; problema de versionamento em AOT/R2R |
| Pilha para objetos | Sem GC; localidade; promoção de campos | Frame maior; referências "mistas" exigem `GC_BYREF` e barriers checked |
| Proibir alocação em loops | Evita crescimento ilimitado/uso incorreto | Perde casos comuns (objetos temporários por iteração) |
| Proibir finalizáveis | Correção (iriam à fila de finalização) | — |
| Limite de tamanho | Controla frame | Objetos médios não se beneficiam |
| Clonar código para garantir não-escape | Resolve casos condicionais | Crescimento de código |
| Programador decide `struct` vs `class` | Previsível | Exige conhecimento; erros de design de tipo |

## Como aplicar na sua linguagem

1. **Dê à linguagem tipos valor de primeira classe** e uma forma explícita de buffer na pilha.
   Isso resolve muitos casos sem nenhuma análise (custo zero de compilador).
2. **Se quiser stack allocation automática, implemente escape analysis depois de inlining**
   (no .NET a fase vem logo depois de `Morph - Inlining`). Sem inlining, quase tudo escapa.
3. **Use um grafo de conexões simples**: nós = variáveis locais que contêm referências;
   arestas = atribuições; marque "escapa" quando o valor vai a campo de heap, retorno,
   argumento de chamada não analisável, captura por closure, ou lançado como exceção. Calcule o
   fecho transitivo.
4. **Registre o motivo** de cada recusa (`"[too large]"`, `"[alloc in loop]"`), é o que torna
   a otimização depurável.
5. **Defina tipos não elegíveis** no runtime: finalizáveis, objetos com identidade especial
   (COM no .NET), tipos que o GC precisa conhecer.
6. **Defina limites**: tamanho máximo, sem loops (a menos que prove que não escapa por
   iteração), arrays só de comprimento constante.
7. **Integre com o GC desde o começo**: decida como reportar referências para a pilha
   (não-GC), para o heap (GC_REF) ou mistas (BYREF) e como o write barrier trata cada caso
   (nenhum / normal / checked). Essa decisão toca IR, GC info e backend.
8. **Planeje a análise condicional** (enumeradores, delegates temporários) se a sua
   linguagem tem abstrações de iteração baseadas em objetos; sem isso, `for-each` sobre
   interfaces sempre aloca. Pode usar devirtualização guiada por perfil + clonagem.
9. **Pense em AOT**: se compilar antecipadamente sem ver o resto do programa, a análise precisa
   ser conservadora; se o seu pipeline tem "linker" de programa inteiro, ele pode fazer
   análise global.
10. **Ofereça uma chave para desligar a otimização** (o .NET tem as configurações
    `JitObjectStackAllocation*` e, em DEBUG, uma faixa por hash de método para bisseção):
    uma otimização desse tipo é arriscada e precisa de chave de desligar.

## Referências

- `docs/design/coreclr/jit/object-stack-allocation.md` - motivação, escape analysis, onde
  rodar, restrições, GC, protótipos e roadmap.
- `docs/design/coreclr/jit/DeabstractionAndConditionalEscapeAnalysis.md`.
- `src/coreclr/jit/objectalloc.cpp` (`DoPhase` linha 188, `CanAllocateLclVarOnStack` 1151,
  `MorphAllocObjNodes` 1326, `AnalyzeParentStack` 1974, clonagem ~3077-4953) e `objectalloc.h`.
- `src/coreclr/jit/jitconfigvalues.h` linhas 722-732 - configuração.
- `src/coreclr/vm/jitinterface.cpp:1877` - `CEEInfo::canAllocateOnStack`.
- `src/coreclr/jit/compphases.h` - `PHASE_ALLOCATE_OBJECTS`.
- `docs/design/features/NonGC-Heap.md` - objetos fora do heap coletável.
- `docs/design/coreclr/jit/ryujit-overview.md` - ordem das fases (Inlining -> Object Stack Allocation).
