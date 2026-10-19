# call vs callvirt vs calli (as três instruções de chamada)

## Mecanismo no .NET

A CIL (ECMA-335) tem três instruções principais de chamada. No JIT elas entram no mesmo caminho de importação: em `src/coreclr/jit/importer.cpp` (~linha 9231) os `case CEE_CALLI`, `CEE_CALLVIRT` e `CEE_CALL` são tratados em sequência e delegam a `impImportCall` (`src/coreclr/jit/importercalls.cpp`).

### call

- Operando: token de método (`methodref`/`methoddef`/`methodspec`).
- Chamada **direta/estática**: o método é escolhido em tempo de compilação, sem consulta à tabela virtual. Usada para métodos estáticos, construtores, chamadas `base.M()` e chamadas não-virtuais de instância.
- Não verifica `this == null` por si só (a C# usa `callvirt` até para métodos não-virtuais justamente para obter essa checagem; conhecimento geral, comportamento do compilador C#).
- Habilita **inlining** direto no JIT.

### callvirt

- Operando: token de método com `this` na pilha.
- Despacho **virtual**: seleciona o override pelo tipo dinâmico do objeto, ou resolve interface.
- Sempre verifica `this != null` (lança `NullReferenceException`), mesmo para métodos não-virtuais.
- Como o runtime implementa o despacho:
  - Métodos virtuais de classe: *vtable slot* via `MethodTable`.
  - Chamadas de interface: **Virtual Stub Dispatch (VSD)** — `docs/design/coreclr/botr/virtual-stub-dispatch.md`, implementado em `src/coreclr/vm/virtualcallstub.cpp`. O doc descreve *lookup stubs*, *dispatch stubs* e *resolve stubs*, com objetivos explícitos de "Working Set Reduction" e "Throughput Parity".
  - O JIT pode **devirtualizar** (ver `docs/design/coreclr/jit/GuardedDevirtualization.md`): troca o `callvirt` por chamada direta guardada por teste do tipo, usando perfil dinâmico.

### calli

- Operando: **assinatura** (`StandAloneSig`), não método. O endereço é o último valor na pilha.
- Uso: ponteiros de função (arquivo 02), `ldftn`/`ldvirtftn`, interop. O IL stub de P/Invoke termina com `calli` para o export nativo (`docs/design/features/source-generator-pinvokes.md`, passo 5).
- A convenção de chamada é a da assinatura: `CORINFO_CALLCONV_DEFAULT` para gerenciado, `CORINFO_CALLCONV_UNMANAGED` para nativo (`corinfo.h`).
- Não permite inlining nem devirtualização (alvo desconhecido, exceto quando o JIT consegue provar o alvo constante).

### Prefixos que modificam as três

`tail.`, `constrained.` (para genéricos e value types: chama diretamente se o tipo implementa o método, senão faz box e `callvirt`), `readonly.`, `no.`. O prefixo `tail.` é tratado em `importercalls.cpp` por `PREFIX_TAILCALL_EXPLICIT` (linhas ~1168 e ~1296); veja arquivo 12.

### Delegates como quarto caso

`Invoke` de delegate é um `callvirt` sobre um método sintético implementado pelo runtime (`method-descriptor.md`: "Delegate methods whose implementation is provided by the runtime"). O `clr-abi.md` (seção ABI portátil, linha ~888) afirma: "Virtual, interface, and delegate calls differ only in how they obtain `pep_ptr`; once the portable entrypoint address is available, they use the same final argument and `call_indirect` sequence."

### Argumentos ocultos

O `clr-abi.md` documenta o que vai junto da chamada: `this`, return buffer, parâmetro genérico oculto (seção "Hidden parameters", "Generics"), `Continuation` para async (seção "Async") e parâmetro secreto de IL stub. Eles mudam conforme o tipo de chamada (por exemplo, métodos genéricos compartilhados recebem contexto via `CORINFO_CALLCONV_PARAMTYPE = 0x80`).


### Exemplo lado a lado (IL ilustrativo)

```text
// C#:  s.ToString();  // s é string (selado)
ldloc s
call   instance string System.String::ToString()     // call: alvo conhecido, inlinável

// C#:  o.ToString();  // o é object
ldloc o
callvirt instance string System.Object::ToString()   // callvirt: despacho virtual

// C#:  f(21);         // f é delegate*<int,int>
ldc.i4 21
ldloc f
calli  int32(int32)                                  // calli: alvo no topo da pilha
```

O compilador C# costuma emitir `callvirt` também para métodos de instância não virtuais, para ganhar a checagem de nulo (conhecimento geral). O JIT converte `callvirt` em chamada direta quando o método não é virtual ou o tipo é exato, mantendo o null-check.

### Como o JIT lida com cada uma (mapa de código)

| Etapa | Onde |
|---|---|
| Importação do opcode | `importer.cpp` (`CEE_CALL`/`CEE_CALLVIRT`/`CEE_CALLI`) e `impImportCall` em `importercalls.cpp` |
| Resolução do token e flags (virtual, final, interface) | consulta ao VM pela interface `corinfo.h` |
| Intrinsic? | `impIntrinsic` (arquivo 05) |
| Candidato a inline? | `impMarkInlineCandidate` (linhas ~1244 e ~1471 de `importercalls.cpp`) |
| Devirtualização | `docs/design/coreclr/jit/GuardedDevirtualization.md` |
| Preparo de argumentos | `morph.cpp`; ver `jit-call-morphing.md` |
| Tail call | `fgMorphPotentialTailCall` (arquivo 12) |
| Chamada não gerenciada | `InlinedCallFrame` (arquivo 04) |

### Como o runtime resolve o alvo de `callvirt`

1. Classe com vtable: o `MethodTable` tem slots; o call site faz carga do slot e chamada indireta (esquema geral; ver `method-descriptor.md` e `type-system.md`).
2. Interface: o call site chama um stub (VSD). O primeiro uso passa por *lookup stub*, que resolve a implementação; depois um *dispatch stub* compara o tipo do objeto com o esperado; se o site vira polimórfico, usa um *resolve stub* com cache. Fonte: `virtual-stub-dispatch.md` (seções "Lookup Stubs", "Dispatch Stubs", "Resolve Stubs").
3. Genéricos compartilhados: precisam do argumento de contexto oculto (`CORINFO_CALLCONV_PARAMTYPE`).

### Propriedades que sua linguagem deve decidir

| Pergunta | Resposta .NET |
|---|---|
| Chamar método de instância em `null`? | `callvirt` lança NRE; `call` não verifica |
| O alvo de `calli` pode ser verificado? | Não; a assinatura é confiança |
| O despacho de interface muda após carga de tipos? | Sim; os stubs aprendem e podem ser reparados |
| `base.M()` | sempre `call` (nunca virtual) |

### Efeito sobre a otimização

- `call` e `callvirt` com alvo exato permitem inlining, propagação de constantes e análise de escape (`object-stack-allocation.md`, `DeabstractionAndConditionalEscapeAnalysis.md` em `docs/design/coreclr/jit/`).
- `calli` com alvo constante pode ser tratado como `call` pelo JIT se ele provar o endereço (otimização oportunista).
- Virtual com perfil dinâmico: PGO alimenta devirtualização guardada (`docs/design/features/DynamicPgo.md`).


### Glossário

| Termo | Significado |
|---|---|
| VSD | Virtual Stub Dispatch, despacho de interface por stubs |
| Vtable slot | posição de um método virtual na tabela do `MethodTable` |
| Devirtualização | trocar chamada virtual por direta quando o tipo é conhecido |
| Constrained call | prefixo para chamar método virtual em tipo genérico sem box desnecessário |

### Testes sugeridos

1. Chamar método de instância com receptor nulo por `call` e por `callvirt`, comparando o comportamento.
2. Chamar a mesma interface com 1, 2 e muitos tipos de receptor no mesmo call site.
3. Verificar inlining de `call` direto contra `callvirt` de método selado.
4. Chamar um `calli` com assinatura errada de propósito e observar a falha (comportamento indefinido; apenas para testes isolados).

## Por quê

- **Separar o que é resolvido no compile-time do que é no runtime**: `call` permite otimização máxima; `callvirt` entrega polimorfismo; `calli` entrega indireção arbitrária.
- **`calli` independe de metadados de método**: serve a qualquer fonte de endereço (nativo, `ldftn`, geradores de IL).
- **`callvirt` fixa a semântica do null-check no IL**, e o JIT remove a verificação quando provável não-nula.
- VSD existe porque tabelas de interface por classe custam memória; os stubs resolvem sob demanda e aprendem o tipo recorrente (monomórfico).

## Trade-offs

| | call | callvirt | calli |
|---|---|---|---|
| Custo | mínimo | vtable/VSD | indireção + sem inline |
| Inlining | sim | só após devirtualizar | não |
| Null check | não | sim | não |
| Segurança | verificada por token | verificada | assinatura não verificada contra o alvo |
| Reflexão/ferramentas | fácil | fácil | difícil (alvo opaco) |

- VSD troca throughput de pior caso (polimórfico) por memória e simplicidade de layout.
- `callvirt` em tipos selados/valores é custo desperdiçado se o compilador não emitir `call`.
- `calli` perde informação para o linker/AOT: o compilador AOT não vê o grafo de chamadas.

## Como aplicar na sua linguagem

1. Mantenha **três formas semânticas** no IR da sua linguagem: direta (símbolo conhecido), virtual/interface (por tipo dinâmico) e indireta (por valor). Mesmo que o front-end esconda, o backend ganha muito de saber qual é.
2. Defina explicitamente o contrato de null-check (o .NET o amarra a `callvirt`); a alternativa é checar em todo acesso de membro.
3. Para interfaces, escolha entre (a) tabela de interface por tipo (rápido, caro em memória), (b) cache inline/stub (VSD), (c) hashing de selector. O doc de VSD enumera os objetivos e é uma boa referência de requisitos.
4. Implemente **devirtualização guardada** cedo; o ganho é de inlining, não da chamada.
5. Dê a `calli`-equivalente uma assinatura estática obrigatória, incluindo convenção, para que o backend gere o código correto.
6. Decida se argumentos ocultos (contexto genérico, return buffer, continuation) fazem parte do ABI da linguagem ou são do backend — o .NET os expõe como parte do ABI e isso complica ports.

## Referências

- `src/coreclr/jit/importer.cpp` (`CEE_CALL`/`CEE_CALLVIRT`/`CEE_CALLI`)
- `src/coreclr/jit/importercalls.cpp` (`impImportCall`, prefixos de tail call)
- `src/coreclr/vm/virtualcallstub.cpp`, `virtualcallstub.h`
- `docs/design/coreclr/botr/virtual-stub-dispatch.md`
- `docs/design/coreclr/jit/GuardedDevirtualization.md`
- `docs/design/coreclr/botr/clr-abi.md`, `method-descriptor.md`
- `src/coreclr/inc/corinfo.h` (`CorInfoCallConv`)
- `docs/design/features/source-generator-pinvokes.md`
