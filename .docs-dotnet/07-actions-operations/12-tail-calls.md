# Tail calls (.tail) e recursão

## Mecanismo no .NET

A CIL tem o prefixo `tail.` antes de `call`/`callvirt`/`calli`. Ele significa: "esta chamada está em posição de cauda e **deve** ser executada como tail call para ser correta" (em contraste com otimização oportunista). Documentação central: `docs/design/features/tailcalls-with-helpers.md`.

Do doc: tail call é essencial para linguagens funcionais onde "recursion is typically the only way of looping over data structures"; como nem todas as linguagens precisam, ".NET allows languages to request this optimization to be done on a per-call basis" pelo prefixo. O prefixo só é legal em chamadas em posição de cauda e com condições da ECMA-335.

### Três situações

1. **Tail call explícito** (`tail.`): o importador do JIT marca `isExplicitTailCall` (`PREFIX_TAILCALL_EXPLICIT`, `src/coreclr/jit/importercalls.cpp` ~linhas 1168 e 1296). O JIT **é obrigado** a honrar.
2. **Tail call implícito**: em código otimizado o JIT pode transformar chamadas em cauda sem prefixo (oportunista; sujeito às mesmas condições). (Comportamento geral do RyuJIT; no repositório ver o fluxo de `fgMorphPotentialTailCall`.)
3. **Auto-recursão virando loop**: o clr-abi.md menciona "self-recursive tail calls turned into loops" (seção de profiler, linha ~489: a probe de tailcall "is NOT called for self-recursive tail calls turned into loops").

O C# não emite `tail.`; F# o emite (conhecimento geral, não verificável neste repo).

### Fast tail call

`fgMorphPotentialTailCall` (`src/coreclr/jit/morph.cpp`, ~linha 4441) é chamado "only for calls that the importer has already identified as potential tailcalls. It will do profitability and legality checks and classify which kind of tailcall". Primeiro tenta `fgCanFastTailCall` (~linha 4108), que decide se dá para fazer *epilog + jmp*, reutilizando o frame do chamador. Razões reais de falha, nas strings do código:

- "Argument splitting in callee/caller is not supported on <target>"
- "Profiler is not supported on ARM32"
- "Not enough registers available due to the GS security cookie check"
- "Configuration doesn't allow fast tail calls"
- "Fast tail calls are not performed under tail call stress"
- "Fast tail calls with varargs not supported on Windows ARM/ARM64"
- "Localloc used"
- "Uses NextCallReturnAddress intrinsic"
- "Callee has RetBuf but caller does not."
- "Not enough incoming arg space" (os args do callee ocupam mais pilha do que os do caller recebeu)
- "Callee has a byref parameter" (poderia apontar para o frame que será destruído)
- "Fast tailcalls are not supported on this platform"

Ao verificar a pilha: `calleeArgStackSize` vs `callerArgStackSize = roundUp(lvaParameterStackSize, TARGET_POINTER_SIZE)`.

### Tail call via helpers (fallback)

Se um tail call **explícito** não pode ser rápido, `fgMorphTailCallViaHelpers` (`morph.cpp` ~linha 5248) o transforma usando helpers fornecidos pelo VM (`CORINFO_TAILCALL_HELPERS`, `corinfo.h` ~linha 1890, com flags como `CORINFO_TAILCALL_STORE_TARGET`; o VM gera os stubs em `src/coreclr/vm/tailcallhelp.cpp`/`.h`).

Descrição do doc:

- Objetivo reformulado: "for any arbitrary sequence of tailcalls use constant stack space" (em vez de reciclar o frame).
- Ideia: na **primeira** tail call da sequência, o frame sobrevive e instala um **dispatcher**. As chamadas seguintes **retornam** ao dispatcher anterior, que as executa.
- Três problemas: (1) detectar se o retorno leva a um dispatcher anterior (só isto depende de arquitetura); (2) registrar argumentos e alvo; (3) executar a chamada. (2) e (3) são portáveis.
- Dois **IL stubs** gerados: `IL_STUB_StoreTailCallArgs` (guarda args em **TLS**, via `RuntimeHelpers.AllocTailCallArgBuffer(size, gcDesc)` — `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs` linha ~561, que chama a QCall `TailCallHelp_AllocTailCallArgBufferInternal`) e `IL_STUB_CallTailCallTarget` (lê args, marca o buffer com `TAILCALLARGBUFFER_ABANDONED` e chama o alvo).
- A TLS é necessária porque os args não podem ficar na pilha enquanto o controle retorna ao dispatcher. O buffer é conhecido do GC via **GC descriptor** (refs e byrefs no buffer precisam ser reportados); o marcador "abandoned" evita estender a vida de refs.
- `StubHelpers.NextCallReturnAddress` é um intrinsic JIT que fornece o endereço de retorno da próxima chamada, usado para detectar o dispatcher; chamadores que usam esse intrinsic não podem fazer fast tail call.
- Estrutura `PortableTailCallFrame` com `NextCall` e `TailCallAwareReturnAddress`.
- Quando é `calli` ou método genérico compartilhado, o stub também recebe o ponteiro do alvo.

Exemplo do doc (`IsEven`/`IsOdd` mutuamente recursivos com `tail.`): sem tail call, o empilhamento é O(n); com o mecanismo, O(1).

### Interações

- **Profiler**: antes de preparar argumentos de tail call o JIT insere `CORINFO_HELP_PROF_FCN_TAILCALL` (`clr-abi.md` linha ~489).
- **PInvoke IL stubs**: `clr-abi.md` observa "It looks like we do not support tail calls from PInvoke IL stubs?" (dúvida registrada no doc).
- **WASM**: seção "Tail Calls" (`clr-abi.md` linha ~946) usa `return_call_indirect` e passa o `sp` original ao callee.
- **Async** (`docs/design/specs/runtime-async.md`) e continuation interagem com o ABI; async é excluído de PInvoke/varargs.
- **Debug**: tail calls apagam frames de pilha (consequência observável; stack traces ficam sem o caller).


### Exemplo do doc oficial (mutuamente recursivo)

```csharp
bool IsEven(int x) { if (x == 0) return true;  return /* tail. */ IsOdd(x - 1); }
bool IsOdd (int x) { if (x == 0) return false; return /* tail. */ IsEven(x - 1); }
```

Com fast tail call (jmp) ambos reutilizam o frame. Com a via de helpers, o doc mostra `IL_STUB_StoreTailCallArgs(int x)` chamando `RuntimeHelpers.AllocTailCallArgBuffer(4, null)` e escrevendo `*(int*)argBuffer = x`, e `IL_STUB_CallTailCallTarget(IntPtr argBuffer, ref byte result, PortableTailCallFrame* pFrame)`, que lê o argumento, escreve `TAILCALLARGBUFFER_ABANDONED` e chama `IsOdd`.

### Diagrama do dispatcher (conceitual, baseado no doc)

```text
chamada normal ->  f()  --tail.--> g()   [1o tailcall da sequência]
                    |                      store args (TLS); instala dispatcher;
                    |                      f não libera o frame ainda
                    v
             dispatcher: lê TLS; chama g(args)
                    g()  --tail.--> h()   [tailcall seguinte]
                    |                      store args em TLS; detecta (via NextCallReturnAddress)
                    |                      dispatcher anterior; RETORNA a ele
                    v
             dispatcher: lê TLS; chama h(args) ...   (pilha constante)
```

### Regras da ECMA-335 para `tail.` (conhecimento geral, não verificado no repo)

- A instrução seguinte ao `call` deve ser `ret`.
- A pilha de avaliação deve conter apenas os argumentos da chamada.
- Tipos de retorno compatíveis entre caller e callee (o JIT valida em `impTailCallRetTypeCompatible`, citado em `fgCanFastTailCall` como `assert` em DEBUG).
- A chamada não pode estar dentro de região `try`/`catch`/`filter`/`finally`.
- Ponteiros para o frame do chamador tornam a chamada não verificável.

### Condições por arquitetura

`fgCanFastTailCall` contém limites específicos por alvo ("Argument splitting ... not supported on <target>", "Profiler is not supported on ARM32", "varargs not supported on Windows ARM/ARM64"). Logo, a garantia `tail.` precisa de via de contingência: nem toda ABI permite reaproveitar o frame.

### Observabilidade e testes

- O código cita "tail call stress" como modo de teste ("Fast tail calls are not performed under tail call stress"); o nome da variável de configuração não foi confirmado.
- Em depuração a pilha perde o frame do chamador; por isso `tail.` explícito é opt-in.
- Profilers recebem `CORINFO_HELP_PROF_FCN_TAILCALL`.

### Recursão: estratégias

| Estratégia | Espaço de pilha | Onde no .NET |
|---|---|---|
| Recursão normal | O(n) | padrão |
| Auto-recursão em cauda para loop | O(1) | otimização do JIT (mencionada em `clr-abi.md`) |
| Tail call por jmp | O(1) | `fgCanFastTailCall` |
| Tail call via helpers | O(1) | `fgMorphTailCallViaHelpers` |
| Trampolim manual | O(1) | convenção de biblioteca |

### Lista de verificação de implementação

1. IR com bit "tail obrigatório" por chamada.
2. Verificação de posição de cauda no front-end.
3. Cálculo de espaço de argumentos de saída vs entrada.
4. Via rápida: epílogo + salto.
5. Via geral: buffer de argumentos fora da pilha + dispatcher + descritor para o GC.
6. Reconhecimento de auto-recursão.
7. Hooks de profiler/depurador.
8. Modo de estresse que força a via geral.

### Interações com outros recursos

| Recurso | Interação |
|---|---|
| GC | buffer de argumentos precisa de descritor (refs e byrefs) |
| Exceções | tail dentro de `try` é ilegal; dispatcher precisa preservar semântica |
| Async | continuation é argumento oculto (arquivo 11) |
| Generics compartilhados | alvo pode exigir contexto, passado ao stub de armazenamento |
| Reflexão | `Invoke` não é tail (frames de stub são ocultados, arquivo 08) |

## Por quê

- **Correção**, não só desempenho: algoritmos funcionais recursivos precisam de espaço de pilha constante.
- **Opt-in por chamada** (`tail.`) mantém o custo e a perda de stack trace só onde o front-end quer.
- **Duas vias**: a via rápida é quase gratuita quando possível; a via por helpers garante a **semântica** mesmo quando o ABI impede (args maiores, byrefs).
- **Portabilidade**: a lógica de helpers é em IL, só a detecção do dispatcher depende de arquitetura.

## Trade-offs

- Via de helpers é lenta (TLS, 2 stubs, indireções) e adiciona complexidade ao JIT, VM e GC.
- Via rápida é limitada pelas condições listadas (byref, localloc, argumentos).
- Perda de frames prejudica depuração e stack traces.
- O prefixo exige que o front-end prove "posição de cauda" e cuide de `try`/`finally` (chamadas dentro de região protegida não podem ser tail; condição da ECMA-335).
- Por plataforma, "Fast tailcalls not supported on this platform" retorna ao fallback e custa desempenho.
- Segurança: byref para frame destruído é proibido (daí "Callee has a byref parameter").

## Como aplicar na sua linguagem

1. Decida se tail call é **garantido** pela linguagem (como Scheme/OCaml) ou oportunista (C#). Se garantido, adote a opção do .NET: marcar chamadas em cauda no IR com um bit "must tail".
2. Implemente duas estratégias: **jmp** quando os argumentos do callee cabem na área de args do caller, e um **trampolim/dispatcher** geral como fallback. Se puder, escolha uma única estratégia (trampolim universal) para simplificar, aceitando custo.
3. Se usar trampolim: guarde argumentos fora da pilha (TLS ou registrador de thread) e **informe ao GC** o layout (GC descriptor).
4. Documente as restrições: sem byref para locais, sem `localloc`, sem região protegida, compatibilidade de retorno.
5. Faça auto-recursão em cauda virar **loop** (mais barato que qualquer tail call).
6. Defina o impacto em depuração: marque frames "tail-called" ou preserve metadados.
7. Planeje interação com profilers, async e exceções desde o começo (o `clr-abi.md` precisou documentar exceções).
8. Teste com *tail call stress* (o JIT possui modo que desabilita fast tail para exercitar helpers: "Fast tail calls are not performed under tail call stress").

## Referências

- `docs/design/features/tailcalls-with-helpers.md`
- `src/coreclr/jit/morph.cpp` (`fgCanFastTailCall`, `fgMorphPotentialTailCall`, `fgMorphTailCallViaHelpers`)
- `src/coreclr/jit/importercalls.cpp` (`PREFIX_TAILCALL_EXPLICIT`)
- `src/coreclr/inc/corinfo.h` (`CORINFO_TAILCALL_HELPERS`, `CORINFO_TAILCALL_IS_CALLVIRT`)
- `src/coreclr/vm/tailcallhelp.cpp`, `tailcallhelp.h`
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs`
- `docs/design/coreclr/botr/clr-abi.md` (profiler probes, WASM tail calls)
- `docs/design/coreclr/jit/jit-call-morphing.md`
