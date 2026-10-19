# Escopo léxico, locals e o pool de variáveis locais no IL

Este documento descreve como o .NET representa variáveis locais e escopo no IL, e o que o JIT faz com elas. O ponto central: **o IL não tem escopo léxico**. Ele tem uma tabela plana de locals por método. Escopo é uma noção do compilador de linguagem (e do arquivo de símbolos de debug), não do runtime.

## Mecanismo no .NET

### Assinatura de locals: uma lista plana por método

Cada método IL carrega uma `LocalVarSig` (blob de metadados) que lista os tipos de todos os locals, indexados de 0 a N-1. As instruções acessam por índice: `ldloc`, `stloc`, `ldloca` (ver `src/coreclr/inc/opcode.def`, por exemplo `CEE_LDLOCA` na linha 307, com operando `InlineVar`). Não existe instrução "entrar em escopo" nem "sair de escopo".

O runtime entrega essa assinatura ao JIT pela interface `CORINFO_METHOD_INFO`, campo `locals` (uma `CORINFO_SIG_INFO`). No JIT, `Compiler::lvaInitTypeRef` (`src/coreclr/jit/lclvars.cpp`, em torno das linhas 183-250) faz:

```cpp
lvaCount = info.compLocalsCount = info.compArgsCount + info.compMethodInfo->locals.numArgs;
```

Ou seja: argumentos e locals do IL compartilham **uma única tabela** de descritores (`LclVarDsc`), com os argumentos primeiro (`varNum` começa em `info.compArgsCount` para os locals). Em seguida há um laço que, para cada local da assinatura, chama `getArgType` e `lvaInitVarDsc`.

### Modificador `pinned` faz parte do tipo do local

No mesmo laço de `lclvars.cpp`, o JIT testa `(corInfoTypeWithMod & CORINFO_TYPE_MOD_PINNED) != 0` e, para tipos `CORINFO_TYPE_CLASS` ou `CORINFO_TYPE_BYREF`, marca `varDsc->lvPinned = 1`. Isso corresponde a `ELEMENT_TYPE_PINNED` em `src/coreclr/inc/corhdr.h` (linha 926). Ver o documento 11 (fixed) para detalhes. O que importa aqui: escopo de pinning é "o tempo de vida do local marcado", e como o IL não tem escopo, o compilador de linguagem precisa zerar o local para encerrar o pinning.

### Zero-inicialização: o flag `InitLocals`

`CORINFO_OPT_INIT_LOCALS = 0x00000010` (`src/coreclr/inc/corinfo.h`, linha 713, comentário "zero initialize all variables") é o flag de método que indica que o corpo declarou `.locals init`. Quando ausente, o runtime não garante que os locals comecem zerados (o IL não verificável pode ler lixo). Para locals que contêm referências GC o JIT precisa zerar de qualquer forma, para que o GC não veja ponteiros inválidos; isso é uma exigência de segurança do GC, não de semântica da linguagem.

### O "pool" de variáveis: locals do IL + temporários do JIT

A tabela `lvaTable` do JIT é mais do que os locals do IL. `Compiler::lvaGrabTemp(bool shortLifetime DEBUGARG(const char* reason))` (declarada em `src/coreclr/jit/compiler.h`, linha 4902) aloca novos descritores em tempo de compilação para:

- spill da pilha de avaliação do IL (ver `impSpillStackEntry`, `importer.cpp`);
- temporários de expansão (por exemplo, o booleano `lvaMonAcquired` em métodos `synchronized`, `importer.cpp` ~linha 11991);
- cópias introduzidas por inlining (o inlinee herda `lvaCount` do chamador: `lclvars.cpp` linha 192, `impInlineInfo->InlinerCompiler->lvaCount`).

Note a linha `lvaTableCnt = lvaCount * 2;` (lclvars.cpp ~199): a tabela é pré-dimensionada com folga para temporários. O sufixo "pool" aqui deve ser entendido como essa tabela crescente, onde locals de usuário e temporários são tratados uniformemente depois do import. Há também limite para inlining: `MAX_LV_NUM_COUNT_FOR_INLINING 512` (`compiler.h`, linha 11343).

### Depois do import: escopo desaparece, tempo de vida é recomputado

O JIT recalcula liveness a partir do fluxo de controle (sobre basic blocks), não a partir de escopos. Dois locals de escopos léxicos disjuntos que o compilador de linguagem decidiu *não* reutilizar continuam sendo duas entradas, e o JIT é quem decide registradores/slots de pilha por liveness (ver `docs/design/coreclr/jit/lsra-detail.md` e `variabletracking.md`).

### Onde o escopo existe: metadados de debug

O escopo léxico só sobrevive em informação de debug (PDB / Portable PDB: `docs/design/specs/PortablePdb-Metadata.md`, que define a tabela `LocalScope` e `LocalVariable`). O runtime não usa isso para executar; o JIT apenas recebe mapeamentos de IL offset para native offset (`ee_il_dll.cpp`) e informa o debugger sobre as variáveis (`variabletracking.md`).

### O que o compilador C# faz (convenção fora deste repositório)

O compilador C# (Roslyn) não está neste repositório. O que se pode confirmar *indiretamente* aqui: quando um local precisa viver além do frame (lambda, iterador, async), ele é movido para campos de uma classe/struct gerada, de nome `<>c__DisplayClassN_M` ou `<Metodo>dN` (ver `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs`). Isso é a "promoção" de local para campo (doc 5, 6 e 12). Fora esses casos, é prática de compiladores emitir um índice de local por declaração, e reutilizar slots em builds Release; trate isso como convenção do compilador, não como contrato do runtime.

### Restrições que o JIT impõe ligadas a locals e regiões

- `localloc` é proibido dentro de handler (`importer.cpp`, `BADCODE("Localloc can't be inside handler")`, linha 10335).
- Em métodos async do runtime (`docs/design/specs/runtime-async.md`), locals só são preservados através de suspensão se forem "hoisted"; byrefs, structs ref-like e locals pinned **não são** preservados (leitura depois da suspensão vê `null`/default).
- Locals de tipo byref não podem ser capturados por continuação (`docs/design/coreclr/botr/runtime-async-codegen.md`, seção "Locals handling").

### Exemplo ilustrativo: dois escopos, uma tabela

Código-fonte hipotético (C#-like):

```csharp
void M()
{
    { int a = 1; Use(a); }   // escopo 1
    { int b = 2; Use(b); }   // escopo 2
}
```

Forma possível do IL (ilustrativa; o compilador pode emitir um ou dois slots):

```
.locals init (int32 V_0, int32 V_1)   // ou apenas (int32 V_0) se o compilador reutilizar o slot
IL_0000: ldc.i4.1
IL_0001: stloc.0
IL_0002: ldloc.0
IL_0003: call Use
IL_0008: ldc.i4.2
IL_0009: stloc.1
IL_000a: ldloc.1
IL_000b: call Use
IL_0010: ret
```

Pontos a observar (todos derivados do que o JIT faz, não de Roslyn):

- Não há marcador de fim de escopo. Qualquer separação entre `a` e `b` é invisível ao JIT.
- Após o import, `V_0` e `V_1` viram `LclVarDsc` com `lvaCount` entradas; liveness decide se compartilham registrador.
- Se um dos locals fosse `ldloca` (endereço tomado), o JIT marcaria como "address exposed" e perderia a capacidade de enregistrá-lo, independente do escopo.

### Ciclo de vida de um local no pipeline do JIT (visão resumida)

1. **Descoberta**: `lvaInitTypeRef` lê a assinatura e cria um `LclVarDsc` por argumento/local.
2. **Import**: `importer.cpp` traduz `ldloc/stloc/ldloca` em nós `GT_LCL_VAR`, `GT_STORE_LCL_VAR`, `GT_LCL_ADDR` e adiciona temporários via `lvaGrabTemp`.
3. **Otimizações**: promoção de structs, propagação, eliminação de código morto removem ou dividem locals.
4. **Alocação**: LSRA atribui registradores ou slots de frame, por intervalos de liveness.
5. **Informação de GC/debug**: o JIT emite tabelas de GC (slots rastreados por intervalo) e, se pedido, mapeamento de variáveis para o debugger.

Em nenhuma dessas etapas existe a noção de "bloco léxico".

## Por quê

1. **Simplicidade do formato binário.** Uma tabela plana é trivial de serializar, verificar e transformar. O verificador/JIT não precisa manter uma pilha de escopos.
2. **Liberdade de otimização.** Como o IL não declara tempo de vida, o JIT pode alocar registradores, enregistrar structs (promoção, ver `docs/design/coreclr/jit/first-class-structs.md`) e eliminar locals mortos sem se preocupar com semântica de escopo.
3. **Separação de responsabilidades.** Regras de visibilidade e sombreamento (shadowing) são léxicas e ficam no front-end. O runtime só precisa de identidade (índice) e tipo.
4. **GC precisa de precisão, não de escopo.** O GC é informado, por ponto de programa, de quais slots contêm referências vivas (`jit-gc-info-x86.md`, `clr-abi.md`). Isso é derivado de liveness, não de blocos `{}`.

## Trade-offs

| Decisão | Vantagem | Custo |
|---|---|---|
| Locals planos, sem escopo no IL | Formato simples; JIT livre | Debugger precisa de PDB para mostrar escopo; ferramentas de decompilação reconstroem escopos por heurística |
| `.locals init` como flag por método | Pode-se omitir zeragem em código de alta performance (`SkipLocalsInit`) | Código sem init pode expor lixo; segurança depende de o JIT zerar locals GC |
| Índices de local limitados e numerados | Acesso O(1) | Métodos gigantes geram tabelas grandes; o JIT tem limites de inlining (512 locals) |
| Sem lifetime no IL | Reuso agressivo de registradores | Um local pinned permanece pinned até o compilador zerá-lo (ver doc 11) |
| Locals promovidos a campos em closures/state machines | Semântica correta com lambdas/async | Alocação extra e perda de enregistramento |

Outro trade-off: como a tabela é compartilhada com temporários do JIT, o número final de locals pode ser muito maior que o número declarado, e isso afeta o tempo de compilação (daí o limite para inlining).

## Como aplicar na sua linguagem

1. **Decida onde mora o escopo.** O modelo do .NET sugere: escopo léxico é problema do front-end. Seu IR/bytecode pode ter apenas uma tabela de slots por função. Se sua linguagem tem sombreamento, resolva nomes em tempo de compilação e emita índices.
2. **Se quiser debug decente, emita metadados de escopo separados** (tabela `LocalScope`: intervalo de offsets, lista de variáveis), como o Portable PDB. Não os misture com a semântica de execução.
3. **Defina política de inicialização explícita.** O .NET oferece o flag `InitLocals`. Para uma linguagem nova, a opção mais segura é inicializar sempre, e oferecer um opt-out explícito e escopado para código de desempenho.
4. **Separe "local de usuário" de "temporário do compilador"** desde o início, mas use a mesma estrutura de descritor (como `LclVarDsc`). Isso permite que otimizações tratem ambos uniformemente.
5. **Marque locals com propriedades de tempo de vida no tipo** (como `pinned` na assinatura) apenas se a runtime precisar delas; caso contrário, tenha uma instrução explícita de "fim de escopo" (por exemplo, `scope_end slot`) se você quiser que o pinning ou a finalização determinística terminem exatamente num ponto. O .NET não tem isso: o compilador faz `ldnull; stloc` ou confia no fim do método.
6. **Planeje a captura.** Se sua linguagem terá closures, os locais capturados precisam ser identificáveis no front-end (análise de captura) para serem transformados em campos de um objeto de ambiente (doc 12). O runtime do .NET não sabe disso; é uma transformação puramente de compilador.
7. **Documente limites** (máximo de locals, máximo de argumentos) e como o inliner se comporta, para não ser surpreendido por métodos gerados (state machines grandes).

## Apêndice: decisões de design sobre escopo e locals (checklist)

1. O bytecode terá tabela de locals por função (como `LocalVarSig`) ou registradores virtuais SSA? A primeira é simples; a segunda facilita otimização, mas exige conversão.
2. Os argumentos compartilham a mesma numeração dos locals? No JIT do .NET compartilham `lvaTable`, mas com índices separados no IL (`ldarg` vs `ldloc`).
3. O formato fixa um limite de locals? Decida e documente (o JIT também tem limites internos para inlining).
4. Existirá flag de zero-inicialização por função (`InitLocals`)? Qual o padrão?
5. Como representar "tipo especial de local" (pinned, byref, ref-like)? O .NET usa modificadores na assinatura (`ELEMENT_TYPE_PINNED`, `ELEMENT_TYPE_BYREF`).
6. O endereço de um local pode ser tomado (`ldloca`)? Se sim, o local vira "address exposed" e perde otimizações; ofereça uma regra clara de tempo de vida do endereço (em .NET, `ref` não pode escapar do método por regras do compilador/`ref safety`).
7. Debug: os escopos serão emitidos em arquivo separado (PDB) ou no próprio binário?
8. Temporários do compilador: mesma tabela (como `lvaGrabTemp`) ou tabela separada? Misturar simplifica as otimizações.
9. Locais capturados por closures/state machines: serão reescritos para campos antes do backend (modelo .NET)?
10. Haverá instruções de fim de escopo (para pinning, destruição determinística)? O .NET não tem; confia em atribuições e no fim do frame.

## Apêndice: o que não é do runtime

Para evitar atribuir ao runtime o que é do compilador, uma lista do que **não** está no IL nem no JIT:

- sombreamento de nomes (shadowing) e resolução de identificadores;
- regras de atribuição definida (definite assignment);
- regras de captura em lambdas;
- nomes de variáveis (os nomes só existem em metadados de debug);
- tempo de vida lógico de variáveis de bloco.

Tudo isso é resolvido antes de emitir os índices de locals. O que o runtime enxerga é uma tabela de tipos e uma sequência de instruções que usam índices.

## Referências

- `src/coreclr/inc/opcode.def` (definições de `ldloc`, `stloc`, `ldloca`, `localloc`).
- `src/coreclr/inc/corhdr.h` (linha 926: `ELEMENT_TYPE_PINNED`).
- `src/coreclr/inc/corinfo.h` (linha 713: `CORINFO_OPT_INIT_LOCALS`).
- `src/coreclr/jit/lclvars.cpp` (linhas ~183-250: `lvaCount`, laço de locals, `lvPinned`).
- `src/coreclr/jit/compiler.h` (linha 4902: `lvaGrabTemp`; linha 11343: `MAX_LV_NUM_COUNT_FOR_INLINING`).
- `src/coreclr/jit/importer.cpp` (linha 10335: `localloc` em handler).
- `docs/design/specs/PortablePdb-Metadata.md` (escopos de debug).
- `docs/design/specs/runtime-async.md` e `docs/design/coreclr/botr/runtime-async-codegen.md` (locals hoisted).
- `src/tools/illink/src/ILLink.Shared/DataFlow/CompilerGeneratedNames.cs` (nomes de tipos gerados para locals promovidos).
- `docs/design/coreclr/jit/first-class-structs.md`, `variabletracking.md`, `lsra-detail.md`.

> Nota de confiança: afirmações sobre o comportamento interno do compilador C# (Roslyn) não são verificáveis neste repositório e foram marcadas como convenção.
