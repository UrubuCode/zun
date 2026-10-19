# Modelo de tratamento de exceção em duas passadas (two-pass)

O .NET trata exceções em **duas passadas** sobre a pilha: a primeira *procura* um handler sem alterar a pilha; a segunda *desenrola* a pilha executando `finally`/`fault` até o frame que vai tratar a exceção, e então executa o `catch`. Este documento descreve a implementação gerenciada atual e o que se pode confirmar no repositório.

## Mecanismo no .NET

### 1. Onde mora o despachante

O despachante de exceções moderno é código **C# gerenciado**, compartilhado entre NativeAOT e CoreCLR:

- Arquivo: `src/coreclr/nativeaot/Runtime.Base/src/System/Runtime/ExceptionHandling.cs` (classe `System.Runtime.EH`, método `DispatchEx`, linha ~764).
- O CoreCLR compila esse mesmo arquivo em `System.Private.CoreLib` (`src/coreclr/System.Private.CoreLib/System.Private.CoreLib.csproj`, linha 217) e usa `#if NATIVEAOT` / `#if !NATIVEAOT` para as diferenças.
- Peças nativas de apoio do CoreCLR em `src/coreclr/vm/exceptionhandling.cpp`: `SfiInit` (linha ~3901) e `SfiNext` (~4205) para iterar a pilha; `CallCatchFunclet` (~3074), `CallFinallyFunclet` (~3347), `CallFilterFunclet` (~3374); `InvokeSecondPass` (~4243) e `DispatchExSecondPass` (~4331).

(O caminho antigo baseado em SEH do Windows ainda aparece em `exceptionhandling.cpp` sob `#ifndef HOST_UNIX`, com comentários "First pass (searching)" e "Second pass (unwinding)" no código, mas o desenho descrito abaixo é o gerenciado.)

### 2. Estado: `ExInfo`

Cada exceção em voo tem um `ExInfo` (ver `ExInfo` em `ExceptionHandling.cs` e `src/coreclr/vm/exinfo.h`) com, entre outros: o objeto de exceção (`ThrownException`), `_passNumber` (1 ou 2), um `StackFrameIterator` (`_frameIter`), `_idxCurClause` (cláusula sendo executada) e `_kind` (`ExKind`, com flags como `RethrowFlag` e `InstructionFaultFlag`).

### 3. Primeira passada: busca

Pseudocódigo fiel ao laço de `DispatchEx` (linhas ~770-860):

```
exInfo._passNumber == 1
frameIter.Init(contexto da exceção)
para cada frame (frameIter.Next):
    se atravessou fronteira nativa (reverse P/Invoke): para
    se a exceção foi "interceptada" (debugger): para
    UpdateStackTrace(...)                        // acrescenta o frame ao stack trace
    se FindFirstPassHandler(exc, startIdx, ...): // procura cláusula aplicável
        handlingFrameSP = frameIter.SP
        pCatchHandler   = endereço do handler
        break
se nenhum handler: OnUnhandledExceptionViaClassLib + FailFast
```

`FindFirstPassHandler` (linha ~1029) percorre as cláusulas EH do método com `RhpEHEnumInitFromStackFrameIterator`/`RhpEHEnumNext`:

- Considera apenas cláusulas `RH_EH_CLAUSE_TYPED` e `RH_EH_CLAUSE_FILTER` (enum `RhEHClauseKind` na linha 30; `FAULT` e o resto são ignorados nesta passada).
- Verifica se o offset de código atual está dentro do `try` da cláusula.
- Para `TYPED`: testa se o tipo da exceção é compatível com o tipo da cláusula.
- Para `FILTER`: **chama o funclet de filtro** (`RhpCallFilterFunclet`) ainda com a pilha intacta. Se retornar verdadeiro, o handler é aceito.
- No NativeAOT, a chamada do filtro é envolta em `try { ... } catch when (true) { }`, com o comentário "Prevent leaking any exception from the filter funclet": uma exceção lançada dentro do filtro é descartada e conta como "não trata".

Nada é desenrolado nem executado em `finally` aqui. O resultado da primeira passada é um par: **qual frame trata** (`handlingFrameSP`) e **qual cláusula** (`catchingTryRegionIdx`).

### 4. Segunda passada: desenrolar

Linhas ~895-960:

```
exInfo._passNumber = 2
RhpSetThreadDoNotTriggerGC()      // comentário: o stackwalker não tolera GC aqui
frameIter.Init(...)               // reinicia a partir do ponto do throw
enquanto frameIter.SP <= handlingFrameSP:
    se frame == handlingFrameSP:
        InvokeSecondPass(exInfo, startIdx, catchingTryRegionIdx)  // só cláusulas "dentro" do catch alvo
        break
    InvokeSecondPass(exInfo, startIdx)                            // todos os finally/fault do frame
RhpCallCatchFunclet(exceptionObj, pCatchHandler, ...)             // executa o catch e retoma
```

`InvokeSecondPass` (linha ~1188) percorre as cláusulas do frame e, para cada uma de tipo `RH_EH_CLAUSE_FAULT` (que representa tanto `finally` quanto `fault` na tabela de runtime) cujo `try` contém o PC, chama `RhpCallFinallyFunclet`. A ordem das cláusulas garante a ordem correta: "Because of the order of the clauses, we know this is the most containing".

Pontos importantes extraídos dos comentários do código:

- **Sem GC entre finallys**: "We need to suppress GC 'in-between' calls to finallys in this loop because we do not have the correct next-execution point live on the stack". O estado é resetado quando o laço termina.
- **Retomada**: `RhpCallCatchFunclet` executa o catch; o catch devolve o endereço de retomada (ver `clr-abi.md`) e a execução continua dali. Nunca retorna ao despachante ("unreachable").

### 5. Exceções aninhadas, rethrow e "collided unwind"

O parâmetro `startIdx`/`uExCollideClauseIdx` (retornado por `SfiNext`/`frameIter.Next`) permite que uma exceção lançada **dentro de um handler em execução** continue a busca *a partir da cláusula seguinte*, ignorando as cláusulas que já estavam em processamento. Trecho em `FindFirstPassHandler`: "Skip to the starting try region. This is used by collided unwinds and rethrows to pickup where the previous dispatch left off." e a condição extra `ehClause._isSameTry` no CoreCLR.

O comentário em `exceptionhandling.cpp` (~linha 2592) também registra que os `ExInfo` são "colapsados na segunda passada quando uma exceção aninhada escapa", o que faz perder informação sobre o funclet representado pelo tracker colapsado: um detalhe de implementação que afeta o stack walker (`ExInfo::GetCallerSPOfParentOfNonExceptionallyInvokedFunclet`).

### 6. Fronteiras nativas

O despachante **não propaga exceções através de código nativo**: se o iterador cruza um frame de reverse P/Invoke (`unwoundReversePInvoke`), a busca para ("For EH dispatch, we disallow dispatching exceptions across native code"). No NativeAOT, com `FEATURE_OBJCMARSHAL`, a biblioteca pode fornecer um callback de propagação; senão o resultado é exceção não tratada e FailFast.

### 7. Exceção não tratada

Se a primeira passada não encontra handler (e não há interceptação/propagação), `OnUnhandledExceptionViaClassLib` é chamado e depois `UnhandledExceptionFailFastViaClasslib(RhFailFastReason.UnhandledException, ...)`. Importante: a segunda passada **não** é executada nesse caso ("We FailFast above if the exception goes unhandled. Therefore, we cannot run the second pass", assert na linha ~877), ou seja, `finally` blocks não rodam em exceção não tratada nesse caminho.

### 8. Interação com o filtro e o debugger

- O filtro roda na 1ª passada com a pilha completa: o debugger e `when` (C#) enxergam o estado do ponto do throw.
- `src/coreclr/vm/exceptionhandling.cpp` ~linha 2978: "Notify the debugger that we are on the first pass for a managed exception."
- O BOTR `docs/design/coreclr/botr/exceptions.md` (linhas ~224-226) lembra, para SEH nativo no runtime, que filtros rodam na primeira passada e que destrutores/holders só rodam na segunda, portanto o filtro "must be prepared for any random, and likely inconsistent, state".

## Por quê

1. **Semântica de filtro correta.** Se a pilha fosse desenrolada durante a busca, o filtro executaria sem o estado do ponto do throw, e não seria possível decidir "não tratar" sem ter já destruído frames.
2. **Exceções não tratadas diagnosticáveis.** Como nada foi desenrolado quando a busca falha, o processo pode gerar dump/relato com a pilha original intacta (no caminho de exceção não tratada que cai em FailFast).
3. **Execução de finally somente quando há garantia.** Executar finallys apenas se um handler existe evita trabalho (e efeitos colaterais) inútil em programas que vão terminar.
4. **Pareamento com o formato de tabelas.** O mesmo conjunto de cláusulas serve às duas passadas: tipadas/filtros são vistos na 1ª, fault/finally na 2ª.
5. **Integração com o GC.** Durante a 2ª passada o desenrolamento aplica semântica específica de funclets; por isso o código desliga GC no despachante (`RhpSetThreadDoNotTriggerGC`).

## Trade-offs

| Aspecto | Vantagem | Custo |
|---|---|---|
| Duas varreduras da pilha | Semântica de filtro e diagnóstico | Lançar envolve varredura de pilha, consulta de tabelas e chamadas de funclets, portanto é bem mais caro que um branch (magnitude não medida aqui); exceção não deve ser fluxo de controle normal |
| Filtros na 1ª passada | `when` com estado completo | Pode ver estado inconsistente; ordem de efeitos observável (filtro de frame externo roda antes de finally de frame interno) |
| Despachante em código gerenciado | Reuso entre CoreCLR e NativeAOT; mais fácil de evoluir | Código crítico precisa evitar GC e alocações; bootstrap delicado (comentários sobre GC proibido) |
| Funclets para handlers | Cada handler é uma função; unwinder nativo padrão | ABI complexa (registradores, endereço de retomada, clonagem de cláusulas) |
| Não cruzar código nativo | Segurança e previsibilidade | Exceções gerenciadas não atravessam frames nativos: precisa de marshalling ou falha rápida |
| Sem 2ª passada em exceção não tratada | Falha rápida, preserva estado | `finally` não roda em crash |

A ordem observável "filtro externo antes do finally interno" é uma consequência direta do modelo e é um erro comum de entendimento: em

```csharp
try { try { throw X; } finally { Log("F"); } }
catch (Exception e) when (Log("W")) { }
```

o filtro `W` é avaliado antes do `finally` interno `F` (deduzido do funcionamento das duas passadas; não há teste citado aqui).

## Como aplicar na sua linguagem

1. **Escolha entre uma e duas passadas conscientemente.**
   - Duas passadas permitem filtros/condições na captura e diagnósticos sem perda da pilha.
   - Uma passada (desenrolar enquanto busca, estilo C++ com destrutores) é mais simples, mas condição de captura precisa ser avaliada depois de desenrolar, ou não existir.
2. **Separe "busca" de "execução de limpeza" no seu formato de tabela**: marque cláusulas como "captura" vs. "limpeza" (o .NET usa `TYPED/FILTER` vs. `FAULT`, com `finally` modelado como fault + saída normal).
3. **Guarde o resultado da busca** (frame alvo e índice da cláusula) para a segunda passada: evita reavaliar filtros (que podem ter efeitos colaterais).
4. **Defina o que acontece com exceções dentro de handlers/filtros**: no .NET, exceção em filtro é engolida; em finally/catch cria uma nova exceção com busca retomando após a cláusula corrente ("collided unwind").
5. **Projete o retorno do catch**: o handler devolve um endereço de retomada ao runtime; a execução não "retorna" ao despachante.
6. **Proíba GC/alocação/suspensão no despachante ou garanta que seja seguro**; o .NET desabilita GC durante a 2ª passada.
7. **Defina política para exceções não tratadas e fronteiras nativas** (falhar rápido, ou propagar via callback).
8. **Meça custo**: planeje que `throw` seja caro; ofereça formas sem exceção (por exemplo, `TryParse`, resultados de erro) para fluxo esperado.

## Apêndice: linha do tempo de um throw

Cenário: `Main` chama `A`, que chama `B`; `B` lança `X`. `A` tem `try { B(); } finally { F(); }`; `Main` tem `try { A(); } catch (X) when (P()) { H(); }`.

| Passo | Passada | Ação |
|---|---|---|
| 1 | - | `throw` vira chamada ao helper de throw (`CORINFO_HELP_THROW`, `corinfo.h` ~397); o resto do caminho até `DispatchEx` (criação do `ExInfo`, definição de `_passNumber = 1`) é simplificado aqui |
| 2 | 1 | Frame de `B`: nenhuma cláusula contém o PC, segue |
| 3 | 1 | Frame de `A`: a cláusula é `finally` (kind `FAULT` na tabela de runtime), ignorada na 1ª passada |
| 4 | 1 | Frame de `Main`: cláusula de filtro cobre o PC; `RhpCallFilterFunclet` executa `P()` com a pilha ainda intacta |
| 5 | 1 | `P()` devolve verdadeiro: `handlingFrameSP` = SP de `Main`; `pCatchHandler` = endereço de `H` |
| 6 | 2 | Reinicia o iterador; frame de `B`: nada a limpar |
| 7 | 2 | Frame de `A`: `InvokeSecondPass` chama `F()` via `RhpCallFinallyFunclet` |
| 8 | 2 | Frame de `Main` (= `handlingFrameSP`): `InvokeSecondPass` parcial (`catchingTryRegionIdx`) |
| 9 | - | `RhpCallCatchFunclet` executa `H()`; o endereço de retomada que `H` devolve é o destino do `leave` |

Se `P()` devolvesse falso, a 1ª passada continuaria procurando em frames externos; se nenhum aceitar, `OnUnhandledExceptionViaClassLib` e FailFast, sem executar `F()`.

## Apêndice: perguntas de design para o seu modelo de exceções

1. **Duas passadas ou uma?** Duas dão semântica de filtro e diagnóstico de crash; uma é mais simples e comum em runtimes mais leves.
2. **A exceção é um objeto do heap ou um valor?** No .NET é um objeto (`Exception`) com stack trace acumulado durante a 1ª passada (`UpdateStackTrace`).
3. **Quem acumula o stack trace?** O despachante, frame a frame, na 1ª passada.
4. **O que acontece com exceções dentro de finally/catch/filter?** Substituição (finally/catch) ou descarte (filtro).
5. **Qual o custo aceitável do throw?** Deve ser documentado como "excepcional".
6. **Como interagir com o GC?** Durante a 2ª passada o .NET proíbe GC no despachante (`RhpSetThreadDoNotTriggerGC`).
7. **Como tratar fronteiras de linguagem (nativo)?** O .NET não propaga por código nativo (reverse P/Invoke) e oferece callbacks/FailFast.
8. **Exceções assíncronas (abort, stack overflow, access violation)?** O .NET converte falhas de hardware em exceções gerenciadas (`ExKind.InstructionFaultFlag`, `HwExceptionCode` em `ExceptionHandling.cs`); stack overflow e corrupção de estado seguem política própria (`IsProcessCorruptedStateException` no caminho Windows de `exceptionhandling.cpp`).
9. **Interceptação por depurador?** O despachante verifica `isExceptionIntercepted` em ambas as passadas.
10. **Rethrow:** preserva o stack trace original; o despachante retoma a busca de onde parou (`RethrowFlag`).

## Referências

- `src/coreclr/nativeaot/Runtime.Base/src/System/Runtime/ExceptionHandling.cs`: `DispatchEx` (~764), `FindFirstPassHandler` (~1029), `InvokeSecondPass` (~1183-1240), `RhEHClauseKind` (~30), `ExKind` (~478).
- `src/coreclr/System.Private.CoreLib/System.Private.CoreLib.csproj` (linha 217, inclusão do arquivo acima).
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/ExceptionServices/ExceptionHandling.CoreCLR.cs`.
- `src/coreclr/vm/exceptionhandling.cpp`: `SfiInit`, `SfiNext`, `InvokeSecondPass`, `DispatchExSecondPass`, `CallCatchFunclet`, `CallFinallyFunclet`, `CallFilterFunclet`; comentários "First pass (searching)" (~487) e "Second pass (unwinding)" (~515).
- `src/coreclr/vm/exinfo.h`, `exinfo.cpp`, `exstate.h`.
- `docs/design/coreclr/botr/exceptions.md`, `docs/design/coreclr/botr/clr-abi.md` (funclets), `docs/design/coreclr/botr/stackwalking.md`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/ExceptionServices/ExceptionHandling.cs` (handler global de exceções não tratadas).
