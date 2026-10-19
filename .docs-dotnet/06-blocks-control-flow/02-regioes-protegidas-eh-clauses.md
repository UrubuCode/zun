# Regiões protegidas no IL: try/catch/finally/filter (EH clauses)

No IL do .NET, `try`/`catch`/`finally` **não são instruções**. São **intervalos de offsets** declarados numa tabela separada do corpo do método (a "exception section"). Esta é a decisão de projeto mais importante a entender se você for projetar bytecode para uma nova linguagem.

## Mecanismo no .NET

### 1. A tabela de cláusulas (formato no arquivo)

Definida em `src/coreclr/inc/corhdr.h`:

```c
typedef enum CorExceptionFlag
{
    COR_ILEXCEPTION_CLAUSE_NONE,                // typed handler (catch)
    COR_ILEXCEPTION_CLAUSE_FILTER    = 0x0001,  // entrada de filtro
    COR_ILEXCEPTION_CLAUSE_FINALLY   = 0x0002,  // finally
    COR_ILEXCEPTION_CLAUSE_FAULT     = 0x0004,  // finally só em caminho excepcional
    COR_ILEXCEPTION_CLAUSE_DUPLICATED= 0x0008,  // obsoleto
    COR_ILEXCEPTION_CLAUSE_SAMETRY   = 0x0010,  // mesma região try da anterior
} CorExceptionFlag;
```

Cada cláusula "fat" (`IMAGE_COR_ILMETHOD_SECT_EH_CLAUSE_FAT`, `corhdr.h` ~linha 1161) contém:

| Campo | Significado |
|---|---|
| `Flags` | tipo de handler |
| `TryOffset`, `TryLength` | intervalo protegido, em bytes de IL |
| `HandlerOffset`, `HandlerLength` | intervalo do handler |
| `ClassToken` **ou** `FilterOffset` | união: token do tipo capturado (catch) ou offset do início do bloco de filtro |

Existe também a versão "small" (`IMAGE_COR_ILMETHOD_SECT_EH_CLAUSE_SMALL`, campos de 16/8 bits) para métodos pequenos. A mesma forma é repassada ao JIT em `CORINFO_EH_CLAUSE` (`src/coreclr/inc/corinfo.h`, linha 1699), com `CORINFO_EH_CLAUSE_FLAGS` (linha 823) espelhando os bits acima.

Consequência: **as cláusulas se ordenam do mais interno para o mais externo**; a ordem é significativa porque o runtime procura "a primeira cláusula cuja região contém o PC".  (Ver a busca linear em `FindFirstPassHandler`, `src/coreclr/nativeaot/Runtime.Base/src/System/Runtime/ExceptionHandling.cs`, linha 1029, que percorre `RhpEHEnumNext`; o comentário em `InvokeSecondPass` diz "Because of the order of the clauses, we know this is the most containing".)

### 2. Os quatro tipos de handler

- **catch tipado**: `ClassToken` identifica o tipo; casa por verificação de tipo no 1º passo.
- **filter**: bloco de código (`FilterOffset`) que roda **durante a primeira passada**, sobre a pilha ainda intacta, e termina com `endfilter` devolvendo um inteiro (não zero = "tratarei").
- **finally**: roda sempre que a região é abandonada, tanto por saída normal (`leave`) quanto por exceção.
- **fault**: como o finally, mas só roda no caminho excepcional. Em IL, `endfault` é um alias de `endfinally` (`src/coreclr/inc/opcode.def`, linha 353: `OPALIAS(CEE_ENDFAULT, "endfault", CEE_ENDFINALLY)`).

### 3. Instruções relacionadas

De `opcode.def`:

- `leave` / `leave.s` (linhas 258-259): única forma legal de sair de um `try` ou de um `catch` por fluxo normal. **Esvazia a pilha de avaliação.**
- `endfinally` (257): termina um finally/fault; retorna ao ponto que o chamou.
- `endfilter` (311, `PopI`): termina o filtro e consome o veredito.
- `throw` (159) e `rethrow` (320): `rethrow` só é válido dentro de um catch (o importer rejeita: `BADCODE("rethrow outside catch")`, `importer.cpp` ~linha 11076).

### 4. Regras estruturais verificadas pelo JIT

No importador/construtor de basic blocks (`src/coreclr/jit/fgbasic.cpp`) há validações em tempo de compilação, todas `BADCODE(...)`:

- `try block length <=0` (linha ~3466), `try offset is > codesize`, `handler offset > codesize`, `filter offset > codesize`;
- `endfinally / endfault / endfilter in nested try block` (linha ~4141): essas instruções precisam estar no nível de aninhamento correto;
- `Exception catch type is Null` (linha ~3762);
- `Evaluation stack must be empty on entry into a try block` (`importer.cpp`, linha 12407);
- `localloc` proibido em handler (`importer.cpp`, linha 10335);
- `endfilter outside filter` e `stack must be 1 on end of filter` (`importer.cpp`, ~7252-7272).

Ou seja: o IL é **estruturado** por regiões, mesmo que o corpo dentro dela seja um grafo arbitrário de branches. Entradas só podem ocorrer no início do `try` (não se pode saltar para o meio), e saídas só por `leave`/`throw`/`endfinally`/`endfilter`.

### 5. Representação no JIT: `EHblkDsc`

Em `src/coreclr/jit/jiteh.h` (linha 80) a tabela vira:

```cpp
struct EHblkDsc {
    BasicBlock* ebdTryBeg;  BasicBlock* ebdTryLast;
    BasicBlock* ebdHndBeg;  BasicBlock* ebdHndLast;
    union { BasicBlock* ebdFilter; unsigned ebdTyp; };
    EHHandlerType ebdHandlerType;       // CATCH, FILTER, FAULT, FINALLY, FAULT_WAS_FINALLY
    unsigned short ebdEnclosingTryIndex;
    unsigned short ebdEnclosingHndIndex;
    unsigned short ebdFuncIndex;        // após criação de funclets
    ...
};
```

Pontos notáveis do próprio comentário do código:
- Existem dois índices de aninhamento distintos: "enclosing try" e "enclosing handler". Um bloco pode estar simultaneamente dentro de um try e dentro de um handler de outra cláusula.
- Catch e filter que protegem a **mesma** região try ("mutually protect") têm try regions aninhadas, mas o catch interno **não** está dentro do catch externo: se ele lançar, o externo não o captura.
- `ExFlowBlock()`: para efeitos de fluxo de controle, o JIT modela o início do handler como sucessor excepcional do try, ou o filtro, se houver; e a transição filtro para handler é tratada como fluxo normal.

### 6. Funclets: como o código nativo reflete as regiões

Segundo `docs/design/coreclr/botr/clr-abi.md` (seção "Funclets", linha 235): handlers (finally, fault, filter, filter-handler e catch) são extraídos em **funclets**, tratados pelo SO como funções de primeira classe (com seu próprio `RUNTIME_FUNCTION`), alocados contiguamente com a função pai e compartilhando GC info. A única forma de entrar num funclet é por **chamada**: pelo despachante de exceções do VM, ou pelo código normal, no caso de saída não-excepcional ("local unwind" / non-local exit), que em IL é sempre um `leave`.

Valores de retorno de funclets (`clr-abi.md`, ~linhas 365-371): filtro devolve booleano; catch e filter-handler devolvem o **endereço de retomada**; finally e fault não devolvem nada.

O comentário no início de `src/coreclr/vm/exceptionhandling.cpp` (linhas ~73-116) explica que, como funclets estão fora de linha, as cláusulas são **duplicadas** para cobrir o intervalo do funclet no método pai (ver `COR_ILEXCEPTION_CLAUSE_DUPLICATED`, hoje marcado como obsoleto em `corhdr.h`).

### 7. O `leave` e os "step blocks"

`Compiler::impImportLeave` (`importer.cpp`, linha 4743) canonicaliza `leave` que atravessa regiões:
- sair de um `catch` vira `BBJ_EHCATCHRET`;
- sair de um `try` protegido por `finally` vira `BBJ_CALLFINALLY` + `BBJ_CALLFINALLYRET`, com blocos intermediários ("step blocks");
- sair de um finally/fault por `leave` é ilegal (comentário no código: "Can't CEE_LEAVE out of a finally/fault handler").

De `clr-abi.md` ("Invoking Finallys/Non-local exits"): o endereço de retorno da chamada ao finally **não pode** estar no corpo do próprio try (senão o VM acharia que o finally protege a si mesmo) e **deve** estar dentro de qualquer região externa protetora (para que exceções lançadas pelo finally sejam tratadas).

### 8. Otimização: finally clonado

`docs/design/coreclr/jit/finally-optimizations.md` e `clr-abi.md` ("Cloned finallys") descrevem que o RyuJIT "inline" o corpo do finally ao longo do fluxo normal de saída do try (queda natural), mantendo o funclet só para o caminho excepcional. Isso é possível justamente porque o IL representa o finally como região, e não como chamada a sub-rotina com semântica opaca.

## Por quê

1. **Zero custo no caminho feliz.** Como as regiões são dados fora do fluxo de instruções, nenhuma instrução é executada ao entrar num `try`. Não há "push handler"/"pop handler" em runtime (diferente de modelos baseados em SJLJ ou registro de frames). O custo só aparece ao lançar.
2. **Tabela pesquisável pelo PC.** O despachante só precisa de um PC e de uma tabela; o desenrolar de pilha usa metadados de unwind de qualquer jeito.
3. **Verificação e análise estáticas.** Regiões bem aninhadas permitem validar estruturalmente (regras acima) e dominar o grafo de fluxo: o JIT sabe que o handler só é alcançável a partir da região protegida.
4. **Flexibilidade de linguagem.** Uma mesma tabela serve C#, F#, VB, C++/CLI, etc. O filtro, por exemplo, existe porque VB e C# (`when`) precisavam avaliar uma condição antes do desenrolar.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Regiões em tabela externa | Sem custo no caminho feliz | Código "fora de linha" para handlers; funclets exigem convenção de ABI complexa |
| Ordem das cláusulas importa | Busca simples | Geradores de código (e emissores de IL) precisam ordenar corretamente; erros dão comportamento sutil |
| `leave` obrigatório | Estrutura explícita; permite inserir `finally` | Pilha esvaziada; compiladores precisam materializar valores em locals antes de sair |
| Filtros na 1ª passada | Condição avaliada com pilha intacta (útil para debug e semântica `when`) | Filtro roda em estado "inconsistente"; código do filtro tem de ser muito defensivo (o BOTR `exceptions.md` linha ~226 recomenda cuidado análogo para SEH nativo) |
| Handlers não podem conter suspensão async (runtime-async) | Simplifica a transformação em state machine | Linguagem precisa lidar com `await` em `catch/finally` por outra via (ver doc 5) |
| Aninhamento exige pilha vazia na entrada do try | Verificação trivial | Expressões como `x = cond ? a : (try ...)` não existem em IL; o front-end tem de reordenar |

## Como aplicar na sua linguagem

1. **Considere o modelo "tabela de regiões" em vez de instruções `try_begin`/`try_end`.** Ele dá custo zero e deixa o otimizador raciocinar sobre regiões como metadados.
2. **Defina o formato da cláusula com o menor conjunto possível**: `{kind, try_start, try_end, handler_start, handler_end, tipo_ou_filtro}`. Os quatro `kind` (catch, filter, finally, fault) são o mínimo prático; `fault` é barato e útil para implementar `using`/`defer` semânticos.
3. **Imponha regras estruturais no verificador**: pilha vazia na entrada do try; saída apenas por `leave`; handlers não são alvos de salto; `rethrow` só em catch; `endfinally` só em finally.
4. **Especifique a ordem das cláusulas** (interno primeiro) e o comportamento de regiões que compartilham o mesmo `try` (flag análogo a `SAMETRY`).
5. **Decida se terá filtro.** Se tiver, documente que ele roda antes do desenrolar e que não pode, por exemplo, suspender nem lançar sem efeitos (no .NET, uma exceção escapando de um filtro é engolida: ver `FindFirstPassHandler`, que no NativeAOT faz `catch when (true)` em volta da chamada do filtro).
6. **Pense em como seu backend implementará handlers**: funclets (como o .NET), blocos inline com tabelas de landing pads (estilo LLVM), ou continuações. O modelo funclet exige que o handler acesse o frame do pai via o registrador de frame (`clr-abi.md`, "Registers on entry to a funclet").
7. **Seja explícito sobre `leave` sobre finally**: se sua linguagem permite `return`/`break` dentro de `try/finally`, seu front-end terá de traduzir em "armazenar valor, `leave` para rótulo de saída, executar finally, continuar" (isso é o que `impImportLeave` espera).

## Apêndice: exemplo de tabela para um try aninhado

Fonte (C#-like):

```csharp
try {                       // T1
    try { A(); }            // T2
    finally { F(); }
}
catch (IOException e) { H(e); }
```

Tabela de cláusulas (ilustrativa; ordem do mais interno para o mais externo):

| # | Flags | Try | Handler | Token |
|---|---|---|---|---|
| 0 | `FINALLY` | corpo de T2 | `F()` | n/a |
| 1 | `NONE` (catch) | corpo de T1 (inclui T2 e seu finally) | `H(e)` | `IOException` |

Com isso: se `A()` lança, a primeira passada percorre a cláusula 0 (ignorada, pois só olha catch/filter), encontra a cláusula 1 (casa o tipo) e escolhe o frame; a segunda passada executa `F()` (cláusula 0) e depois o catch. Se `A()` retorna normalmente, o corpo de T2 termina com `leave` para depois do finally; o JIT gera `BBJ_CALLFINALLY` (ou clona o finally). A ordem interno-primeiro é o que faz o finally rodar antes do catch.

## Apêndice: perguntas de design para o seu formato de regiões

1. **Posição vs. instruções**: regiões por intervalo de offsets (modelo .NET) ou instruções `try_begin/try_end`? O primeiro tem custo zero no caminho feliz e exige código contíguo.
2. **Handlers como funções ou blocos?** Funclets (.NET) facilitam o desenrolar nativo; blocos inline (landing pads) facilitam otimização. Defina o ABI: parâmetros, registrador de frame, valor de retorno (endereço de retomada para catch).
3. **Quais kinds?** Mínimo recomendado: catch tipado, finally, fault; filtro se sua linguagem tem condição na captura.
4. **Regras de ordenação** das cláusulas e de **aninhamento** (o JIT mantém `ebdEnclosingTryIndex` e `ebdEnclosingHndIndex`).
5. **Entrada e saída**: apenas pelo início do try; saída por `leave`; pilha vazia (se a máquina for de pilha).
6. **Regiões que compartilham try** (`SAMETRY`): como representar vários catches para o mesmo try sem duplicar offsets.
7. **Interação com outras features**: suspensão (async), pinning, locks, `localloc` em handlers, inlining de métodos com EH (o JIT recusa inlinear catch: "Inlinee EH clause %u is a catch; we can't inline these (yet)", `fgbasic.cpp` ~3455).
8. **Tamanho do formato**: versão compacta (16/8 bits) e versão larga, como `SMALL` e `FAT`.

## Referências

- `src/coreclr/inc/corhdr.h` (linhas ~1149-1200: `CorExceptionFlag`, `IMAGE_COR_ILMETHOD_SECT_EH_CLAUSE_FAT/SMALL`).
- `src/coreclr/inc/corinfo.h` (linhas 823-830 e 1699: `CORINFO_EH_CLAUSE_FLAGS`, `CORINFO_EH_CLAUSE`).
- `src/coreclr/inc/opcode.def` (`leave`, `endfinally`, `endfilter`, `rethrow`, `throw`, alias `endfault`).
- `src/coreclr/jit/jiteh.h` (`EHblkDsc`, `EHHandlerType`).
- `src/coreclr/jit/fgbasic.cpp` (`fgFindBasicBlocks`, validações `BADCODE`).
- `src/coreclr/jit/importer.cpp` (`impImportLeave`, linha 4743; regras de pilha, linhas 7252-7272, 12407).
- `src/coreclr/nativeaot/Runtime.Base/src/System/Runtime/ExceptionHandling.cs` (`RhEHClauseKind`, `FindFirstPassHandler`, `InvokeSecondPass`); este arquivo é compilado também no CoreCLR (`src/coreclr/System.Private.CoreLib/System.Private.CoreLib.csproj`, linha 217).
- `src/coreclr/vm/exceptionhandling.cpp` (comentário de funclets nas linhas ~73-116; `CallCatchFunclet`, `CallFinallyFunclet`, `CallFilterFunclet`).
- `docs/design/coreclr/botr/clr-abi.md` (seção Funclets), `docs/design/coreclr/botr/exceptions.md`, `docs/design/coreclr/jit/finally-optimizations.md`.
