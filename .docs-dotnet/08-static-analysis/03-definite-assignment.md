# Definite assignment (atribuição definida antes do uso)

> Aviso de escopo: a regra de *definite assignment* da linguagem C# (erro CS0165 "uso de variável local não atribuída", CS0170/CS0171 para structs) é implementada no compilador Roslyn, que **não** está em `dotnet/runtime`. O que o repositório permite confirmar são **três camadas posteriores** que lidam com o mesmo problema em níveis mais baixos: (1) o verificador de IL, (2) a flag `.locals init`/`SkipLocalsInit` e (3) a análise de *liveness* do JIT, que descobre variáveis "vivas na entrada" (usadas antes de qualquer definição). Descrevo a regra de C# apenas como conhecimento geral, marcado como tal.

## Mecanismo no .NET

### Camada 0 — regra de linguagem (conhecimento geral, não verificado no repo)

Em C#, uma variável local deve estar *definitivamente atribuída* em todos os caminhos de execução que chegam ao ponto de uso. O compilador faz uma análise de fluxo de dados *forward* com estado por variável `{atribuída, não-atribuída}`; nas junções (merge de `if/else`, loops, `switch`) o estado é a **interseção** (atribuída apenas se atribuída em todos os predecessores). Condições booleanas têm estados "quando verdadeiro / quando falso" (ex.: `if (int.TryParse(s, out var x)) use(x);`). Para `struct`, o rastreio é por campo.

### Camada 1 — verificação de IL

No IL, locais existem sem noção de "atribuição definida" na própria semântica, mas o verificador impõe uma regra equivalente via `initlocals`. Em `src/coreclr/tools/ILVerification/ILImporter.Verify.cs`:

- `_initLocals = _methodIL.IsInitLocals;` (linha ~197).
- Em dois pontos (linhas ~1482 e ~1513), onde o código usa locais, `Check(_initLocals, VerifierError.InitLocals);`.
- `VerifierError.InitLocals` tem a descrição: "initlocals must be set for verifiable methods with one or more local variables".

Logo: **um método só é verificável com locais se o runtime garantir zero-init** — a atribuição "definida" é substituída por uma atribuição **implícita** (zero) feita pelo ambiente. É uma troca: em vez de provar que cada leitura é precedida por escrita, garante-se um valor inicial válido.

Para objetos em construção, a mesma biblioteca implementa uma versão restrita explícita (rastreamento de `this` não inicializado): `_trackObjCtorState`, `_isThisInitialized`, e os erros `ThisUninitStore`, `ThisUninitReturn`, `UninitStack` (`ILImporter.Verify.cs` linhas ~67-69, 155, 1346, 1502-1523, 1691-1697, 1921-1922). O estado `IsThisInitialized` é guardado **por bloco básico** (`basicBlock.IsThisInitialized`), exatamente como uma análise forward de dataflow.

### Camada 2 — `.locals init` e `SkipLocalsInit`

- A flag `initlocals` do método (propriedade `IsInitLocals` do `MethodIL`) diz ao JIT para zerar os locais.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/SkipLocalsInitAttribute.cs` define `SkipLocalsInitAttribute`, aplicável a `Module, Class, Struct, Interface, Constructor, Method, Property, Event` (não a assemblies). O compilador a usa para *não* emitir `.locals init`, deixando locais com lixo de pilha — por isso só é aceito em código `unsafe` e torna o método **não verificável** (consequência direta da regra `InitLocals` acima).
- No JIT, o reflexo é `info.compInitMem` (campo de `compiler.h`; usado em `codegencommon.cpp` ~linha 3860, `codegenarm64.cpp`, `async.cpp`). Quando `compInitMem` é falso, locais sem GC ref não são zerados no prólogo.

### Camada 3 — liveness no JIT: encontrando "usos antes de definição"

O JIT calcula liveness de variáveis (`src/coreclr/jit/liveness.cpp`). A função `Liveness<TLiveness>::MarkMustInitAndEHVars` (~linhas 1361-1425) contém o essencial, confirmado:

```cpp
// Un-init locals may need auto-initialization. Note that the
// liveness of such locals will bubble to the top (fgFirstBB)
// in fgInterBlockLocalVarLiveness()
if (!varDsc->lvIsParam && !varDsc->lvIsParamRegTarget &&
    VarSetOps::IsMember(m_compiler, m_compiler->fgFirstBB->bbLiveIn, varDsc->lvVarIndex) &&
    (m_compiler->info.compInitMem || varTypeIsGC(varDsc->TypeGet())) && !fieldOfDependentlyPromotedStruct)
{
    varDsc->lvMustInit = true;
}
```

Leitura: se uma variável rastreada, que não é parâmetro, está **viva na entrada do primeiro bloco** (`fgFirstBB->bbLiveIn`), isso significa que existe um caminho de uso sem definição prévia — o oposto exato de "definitivamente atribuída". A reação do JIT **não é um erro**, e sim inserir inicialização (`lvMustInit`) se (a) `compInitMem` está ligado ou (b) o tipo contém ponteiro de GC. Para GC refs a regra é obrigatória: o GC não pode ver lixo como referência.

Mais regras do mesmo trecho:

- Variáveis vivas na entrada de handlers de exceção (`exceptVars`) ou na saída de `finally` (`finallyVars`) recebem `lvLiveInOutOfHandler`, porque o fluxo excepcional pode entrar por qualquer ponto do `try`.
- Para saída de `finally`, GC refs que não são argumentos recebem `lvMustInit` — com o comentário "Set lvMustInit only if we have a non-arg, GC pointer".
- Campos de structs promovidos de forma *dependente* não ganham `lvMustInit` (o struct pai é inicializado inteiro).

### Camada 4 — o trimmer: locais num lattice de valores

O scanner de dataflow do ILLink (`src/tools/illink/src/linker/Linker.Dataflow/MethodBodyScanner.cs`) mantém os locais num `LocalVariableStore` (linha ~306: `LocalVariableStore locals = new(methodIL.Variables.Count);`) e simula a pilha IL. Quando não consegue modelar uma operação, empilha `UnknownValue.Instance` (`PushUnknown`, `PopUnknown`). `UnknownValue` (`ILLink.Shared/TrimAnalysis/UnknownValue.cs`) é um `record` imutável singleton: o valor "não sei" do lattice. Não é definite assignment, mas usa a mesma técnica: estado por local + valor-topo para perda de precisão. (Não verifiquei como locais nunca escritos são tratados nesse código; não afirmo.)

### Exemplo de trabalho: três níveis para o mesmo programa

```csharp
int x;
if (cond) x = 1;
Console.WriteLine(x);   // C#: erro CS0165 (conhecimento geral): x não definitivamente atribuída
```

1. **Front-end (Roslyn)**: rejeita. Estado na junção = interseção(`{x atribuída}`, `{x não atribuída}`) = não atribuída.
2. **IL gerado, se o código fosse aceito (por ex. via `SkipLocalsInit` ou IL escrito à mão)**: `x` seria um local lido possivelmente sem escrita. Com `.locals init` o valor é 0; sem, é lixo.
3. **JIT**: a liveness mostra `x` vivo na entrada de `fgFirstBB`; com `compInitMem` ligado, `lvMustInit = true` e o prólogo zera `x`. Se `x` fosse uma referência GC, o JIT zera mesmo sem `compInitMem`.

A moral: cada camada tem uma resposta diferente ao mesmo problema (rejeitar, zerar, ou assumir risco).

## Por quê

- **Segurança de memória**: ler memória não inicializada pode vazar dados ou, para referências, quebrar o GC. Zero-init (camada 2) é a defesa simples; a análise (camadas 0 e 3) evita o custo do zero-init quando comprovadamente desnecessário e dá feedback ao programador.
- **Erro cedo, na fonte**: a regra de linguagem transforma uma classe de bugs em erro de compilação (CS0165), muito antes do IL.
- **Separação de responsabilidades**: o IL não precisa expressar "definitely assigned"; o verificador aceita `initlocals` como contrato; o JIT otimiza o prólogo com liveness.
- **Desempenho**: `SkipLocalsInit` existe porque zerar buffers grandes em `stackalloc` custa; troca-se segurança por velocidade explicitamente.

## Trade-offs

| Estratégia | Vantagem | Desvantagem |
|---|---|---|
| Análise de definite assignment no front-end | Erro preciso, sem custo em runtime | Conservadora: rejeita programas corretos com fluxo complexo (precisa de `= default`/`= null`) |
| Zero-init universal (`.locals init`) | Simples, verificável, determinístico | Custo em métodos com muitos locais/buffers |
| `SkipLocalsInit` | Elimina custo de zeragem | Método não verificável; risco de vazamento de lixo; restrito a `unsafe` |
| Liveness no JIT para decidir `lvMustInit` | Só zera o necessário | Depende de análise precisa; GC refs sempre pagam |
| Estado de `this` por bloco no verificador | Detecta uso de objeto semi-construído | Regra especial; não generaliza para campos individuais |

Observação: a interseção nas junções é conservadora por construção; um loop `while(true)` ou `throw` precisa ser compreendido como fim de caminho (caso contrário gera falsos positivos) — o que conecta este tema ao arquivo 11 (código inalcançável) e ao atributo `DoesNotReturn` (arquivo 02).

## Como aplicar na sua linguagem

1. **Escolha a política**: (a) zero-init implícito para tudo (Go), (b) definite assignment obrigatório (C#, Java), (c) ambos. Se escolher (b), implemente como dataflow *forward* com meet = interseção.
2. **Estado por variável, por bloco básico** — como `basicBlock.IsThisInitialized` no verificador: mantenha um bitset `assigned[var]` na entrada e saída de cada bloco; itere até ponto fixo em CFGs com laços.
3. **Trate condições booleanas com estados duplos** (verdadeiro/falso) para suportar `if (TryGet(out x))`.
4. **Trate structs por campo** se permitir inicialização parcial; do contrário, exija atribuição do struct inteiro.
5. **Marque fim de caminho** (`throw`, `return`, função que nunca retorna) para que a interseção ignore o ramo morto.
6. **Defina um botão de fuga explícito** (`SkipLocalsInit`-like) com semântica clara: "este método pode ler lixo; é seu risco". Faça-o sinalizar o método como não verificável pelo seu verificador.
7. **Garanta zero-init de referências gerenciadas** independentemente do botão de fuga (o JIT faz isso: `varTypeIsGC(...)` força `lvMustInit`).
8. **Considere a análise de construtor separadamente**: acessar `this` antes de a base ser construída é um caso especial relevante em linguagens com herança de classe.

## Apêndice: algoritmo de referência (conhecimento geral) e ligação com o que o repo mostra

### Dataflow forward de definite assignment

```text
Domínio: para cada bloco b, IN[b] e OUT[b] ⊆ Variáveis   (conjunto de variáveis definitivamente atribuídas)
Inicialização: IN[entrada] = parâmetros;  IN[outros] = TODAS as variáveis    # otimista
Transferência: OUT[b] = IN[b] ∪ DEF[b]
Junção:        IN[b] = ⋂ OUT[p]  para todo predecessor p                      # interseção
Iteração: repetir até ponto fixo
Uso: se variável v ∈ USE(instrução) e v ∉ estado_atual: erro "não atribuída"
```

Comparação com o que o repo mostra:

| Aspecto | Definite assignment (linguagem) | `MarkMustInitAndEHVars` (JIT) | `_isThisInitialized` (ILVerify) |
|---|---|---|---|
| Direção | forward | backward (liveness) | forward |
| Pergunta | "todos os caminhos já atribuíram?" | "existe caminho que usa sem definir?" (viva em `fgFirstBB`) | "this já foi inicializado neste bloco?" |
| Domínio | conjunto de variáveis | conjunto de variáveis vivas | um único booleano por bloco |
| Reação | erro de compilação | inserir inicialização (`lvMustInit`) | erro de verificação |

As duas primeiras são **duais**: "definitivamente atribuída" ≡ "não viva na entrada". A liveness do JIT (`liveness.cpp`) calcula `bbLiveIn` por bloco; se a variável aparece em `fgFirstBB->bbLiveIn`, houve algum caminho com uso sem definição — exatamente a negação do definite assignment.

### Casos que a regra de linguagem precisa tratar (checklist)

1. `out`/`ref`: `out` exige atribuição antes de retornar; `ref` exige atribuição antes de passar.
2. Laços `while(true)` sem `break`: o fim é inalcançável, logo o estado após o laço é "vacuamente tudo atribuído".
3. `try/catch/finally`: no `catch`, só o que estava atribuído **antes** do `try` conta; após `finally`, soma-se o que o `finally` atribui. (Isto é o análogo de `lvLiveInOutOfHandler` e `finallyVars` no JIT.)
4. Expressões `&&`, `||`, `?:` com estados "quando verdadeiro/falso".
5. Lambdas: variáveis capturadas precisam estar atribuídas no ponto de criação do lambda.
6. Structs: atribuição por campo; só "inteiro" quando todos os campos o foram.
7. Construtores: ver `ThisUninit*` no verificador.

### Mapeando para a sua linguagem

| Decisão | Opção A | Opção B | Observação do .NET |
|---|---|---|---|
| Valor padrão de locais | Zero-init sempre | Não inicializados | .NET usa `.locals init` por padrão; opção B via `SkipLocalsInit` |
| Erro de uso não atribuído | Erro | Aviso | C# usa erro |
| Referências gerenciadas | Sempre zeradas | Podem ter lixo | JIT força init de GC refs |
| Escape hatch | Atributo por método/módulo | Palavra-chave `uninit` | `SkipLocalsInitAttribute` (Module→Method) |

## Referências

- `src/coreclr/tools/ILVerification/ILImporter.Verify.cs` (`_initLocals`, `_trackObjCtorState`, `_isThisInitialized`, `ThisUninit*`, `InitLocals`)
- `src/coreclr/tools/ILVerification/VerifierError.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/SkipLocalsInitAttribute.cs`
- `src/coreclr/jit/liveness.cpp` (`MarkMustInitAndEHVars`)
- `src/coreclr/jit/codegencommon.cpp` (uso de `compInitMem`, `lvMustInit`)
- `src/tools/illink/src/ILLink.Shared/TrimAnalysis/UnknownValue.cs`; `src/tools/illink/src/linker/Linker.Dataflow/MethodBodyScanner.cs`
- Não presente no repo: implementação C# de definite assignment (Roslyn) e a especificação C# §9 ("Definite assignment").
