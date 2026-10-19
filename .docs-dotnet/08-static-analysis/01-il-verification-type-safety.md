# Verificação de IL e segurança de tipos (type safety)

> Escopo: como o ecossistema .NET verifica que uma sequência de instruções IL é bem tipada e estruturalmente válida, e o que isso ensina para o design de uma linguagem nova com bytecode próprio.

## Mecanismo no .NET

### Contexto histórico e estado atual

O IL do .NET (ECMA-335) é uma linguagem de pilha. Um módulo pode conter IL **válido** (bem formado) mas **não verificável** (não provadamente seguro quanto a tipos). No runtime moderno (CoreCLR) o *loader/JIT não verifica* IL em tempo de execução: a verificação virou uma ferramenta de desenvolvimento. A ferramenta oficial dentro deste repositório é o **ILVerify**:

- `src/coreclr/tools/ILVerify/README.md` — descreve a intenção: "uma ferramenta standalone, multiplataforma e open-source capaz de verificar MSIL com base na ECMA-335". Os usuários principais são autores de compiladores e profilers (quem *emite* IL).
- O README cita o antecessor, o **PEVerify** (.NET Framework), com limitações: preso ao .NET Framework, não verifica `mscorlib.dll`. Daí o ILVerify: sem acoplamento a CoreLib (pode apontar para qualquer assembly de referência), rápido, regras fáceis de adicionar.
- Instalação: `dotnet tool install --global dotnet-ilverify`; uso `ilverify hello.dll -r <dlls de referência>`. Todas as dependências precisam ser passadas explicitamente.

### Estrutura do código

O README divide o projeto em três partes:

| Projeto | Caminho | Papel |
|---|---|---|
| ILVerification | `src/coreclr/tools/ILVerification/` | Biblioteca com a lógica de verificação |
| ILVerification.Tests | `src/coreclr/tools/ILVerification.Tests/` | Testes baseados em arquivos `.il` |
| ILVerify | `src/coreclr/tools/ILVerify/` | CLI sobre a biblioteca |

Arquivos centrais da biblioteca (todos confirmados no diretório):

- `Verifier.cs` — API pública (`Verifier`, `Verify(PEReader)`), usa `IResolver` para resolver assemblies e `ILVerifyTypeSystemContext` como contexto de tipos.
- `ILImporter.Verify.cs` (~2900 linhas) — o "importador" que percorre o IL e simula a pilha.
- `ILImporter.StackValue.cs` — o modelo de valores de pilha (`StackValue`, `StackValueKind`).
- `ILImporter.ExceptionRegions.cs` — regras de blocos try/catch/finally/fault/filter.
- `TypeVerifier.cs` — verificação de metadados de tipo (herança, interfaces etc.).
- `AccessVerificationHelpers.cs` — acessibilidade (visibilidade de membros).
- `VerifierError.cs` — enum com todos os códigos de erro; `Strings.resx` — mensagens.

### Interpretação abstrata da pilha

O núcleo é uma **interpretação abstrata** do IL: em vez de executar, o verificador mantém uma pilha de `StackValue` (um tipo abstrato por slot) e, em cada instrução, desempilha operandos, checa os tipos e empilha o resultado. Exemplo real em `ILImporter.Verify.cs`, `ImportBinaryOperation` (por volta da linha 2138):

```csharp
case ILOpcode.add: case ILOpcode.sub: case ILOpcode.mul: case ILOpcode.div: case ILOpcode.rem:
    CheckIsNumeric(op1); CheckIsNumeric(op2); break;
case ILOpcode.and: case ILOpcode.or: case ILOpcode.xor: ... case ILOpcode.add_ovf: ...
    CheckIsInteger(op1); CheckIsInteger(op2); break;
...
// StackValueKind is carefully ordered to make this work
StackValue result = (op1.Kind > op2.Kind) ? op1 : op2;
if ((op1.Kind != op2.Kind) && (result.Kind != StackValueKind.NativeInt))
    VerificationError(VerifierError.StackUnexpected, op2, op1);
```

Observações verificadas no código:

1. Os tipos de pilha são poucos (o comentário "StackValueKind is carefully ordered" mostra que o enum é ordenado de forma que `max(kind1, kind2)` dê o tipo do resultado). Operações aritméticas só aceitam tipos numéricos; operações como `and/or/xor/div_un/*_ovf` exigem inteiros.
2. Misturar kinds diferentes é erro (`StackUnexpected`), exceto envolvendo `NativeInt`. Há uma exceção documentada: subtração de dois `ByRef` produz `NativeInt`.
3. A compatibilidade de atribuição passa por `CheckIsAssignable(src, dst, error)` (~linha 635) e `IsAssignable`, que consulta o sistema de tipos.

### Fluxo de controle e junção de caminhos

Como o IL tem saltos, a pilha em cada ponto de junção precisa ser compatível entre todos os caminhos. O enum `VerifierError` possui `PathStackUnexpected` ("tipos não compatíveis na pilha dependendo do caminho"). Além disso há uma série de erros estruturais (todos em `VerifierError.cs`):

- `MethodFallthrough` — cair do fim do método sem retornar.
- `FallthroughException`, `FallthroughIntoHandler`, `FallthroughIntoFilter` — queda em blocos de exceção.
- `LeaveIntoTry/Handler/Filter`, `LeaveOutOfFilter/Finally/Fault` — `leave` ilegal.
- `BranchIntoTry/Handler/Filter`, `BranchOutOf*` — saltos que atravessam fronteiras de região protegida.
- `Rethrow`, `Endfinally`, `Endfilter` fora do contexto correto.
- `ReturnFromTry`, `ReturnFromHandler` — `ret` dentro de região protegida.
- `EHClauseOutOfRange` — cláusula de exceção fora do corpo.

Ou seja: **a estrutura do fluxo de controle faz parte da segurança**, não só os tipos dos valores.

### Rastreamento de estado de construtor (`this` não inicializado)

Há uma análise de "estado" dentro do verificador: em construtores de classes (não structs), `_trackObjCtorState` é ligado (`ILImporter.Verify.cs` ~linha 202: `!_methodSignature.IsStatic && _method.IsConstructor && !method.OwningType.IsValueType`). Enquanto `_isThisInitialized` é falso:

- guardar em `this` gera `ThisUninitStore`;
- retornar do `.ctor` gera `ThisUninitReturn` (linha ~1921: `Check(_isThisInitialized || _thisType.IsObject, VerifierError.ThisUninitReturn)`);
- usar `this` na pilha de forma proibida gera `UninitStack` (linha ~155).

A flag vira verdadeira ao encontrar a chamada ao construtor da classe-base (linhas ~1691-1697) e é propagada por bloco básico (`basicBlock.IsThisInitialized`, linha ~1346). É um exemplo de análise de fluxo de dados mínima — um "definite assignment" restrito a um único valor.

### `initlocals`

`ILImporter.Verify.cs` (linhas ~1482 e ~1513) exige `Check(_initLocals, VerifierError.InitLocals)` em certos usos de locais; o enum documenta: "initlocals must be set for verifiable methods with one or more local variables". Ou seja, a verificabilidade depende de o runtime zerar locais (ver arquivo 03).

### Testes como especificação

`src/coreclr/tools/ILVerify/README.md` documenta a convenção: métodos em arquivos `.il` sob `src/coreclr/tools/ILVerification.Tests/ILTests`, nomeados `[FriendlyName]_Valid` ou `[FriendlyName]_Invalid_<ErroEsperado>`. Os testes são pequenos fragmentos de IL escritos à mão, agrupados por funcionalidade ("poucas dezenas de .il, não milhares").

### Catálogo de erros por categoria (de `VerifierError.cs`)

O enum é uma boa "lista de regras de segurança" de um bytecode tipado. Agrupando os membros confirmados:

**Pilha e tipos de operandos**
- `StackUnexpected`, `StackUnexpectedArrayType`, `StackOverflow`, `StackUnderflow`, `UninitStack`.
- `ExpectedIntegerType` ("I, I4 ou I8"), `ExpectedFloatType`, `ExpectedNumericType`, `StackObjRef`, `StackByRef`, `StackMethod`.
- `PathStackUnexpected` e `PathStackDepth`: pilha diferente (em tipo ou profundidade) conforme o caminho.
- `TryNonEmptyStack`, `FinOrFaultNonEmptyStack`, `FilterOrCatchUnexpectedStack`: entrar numa região protegida com pilha incorreta.

**Retorno**
- `ReturnPtrToStack` (retornar byref/TypedReference/ArgHandle/ArgIterator), `ReturnVoid`, `ReturnMissing`, `ReturnEmpty`.

**Objetos, arrays e valores**
- `ExpectedArray` ("array unidimensional com base zero"), `ArrayByRef`, `ByrefOfByref`, `BoxByRef`, `ValueTypeExpected`.
- `CallVirtOnValueType`, `CallVirtOnStatic`, `LdvirtftnOnStatic`, `CallAbstract`, `CtorExpected`, `CtorSig`, `CallCtor`, `LdftnCtor`.
- `InitOnly` ("não alterar campo initonly fora do .ctor").
- `ThrowOrCatchOnlyExceptionType`, `CatchByRef`.

**Acesso**
- `TypeAccess`, `MethodAccess`, `FieldAccess` (visibilidade).

**Chamadas de cauda**
- `TailCall`, `TailByRef`, `TailRet`, `TailRetVoid`, `TailRetType`, `TailStackEmpty`.

**Delegates**
- `DelegateCtor`, `DelegatePattern` ("padrão dup/ldvirtftn/newobj esperado"), `DelegateCtorSigI`, `DelegateCtorSigO`.

**Estrutura do método**
- `CodeSizeZero`, `MethodEnd`, `BadBranch`, `BadJumpTarget`, `MethodFallthrough`.
- `Unverifiable` ("instrução não pode ser verificada"), `Volatile`/`Unaligned` (prefixos sem a instrução seguinte correta), `InitLocals`.

Muitas entradas aparecem *comentadas* (`//E_...`) no arquivo: são códigos herdados do verificador antigo do .NET Framework que não foram (ainda) reimplementados. Isso é um sinal útil: o verificador atual é intencionalmente um subconjunto.

### Resultado e API

`VerificationResult.cs` representa cada achado (erro, método, tipo, argumentos formatados) e `Verifier.Verify(PEReader)` devolve `IEnumerable<VerificationResult>`. `VerifierOptions` (referenciado em `Verifier.cs`) configura o comportamento. A API é *pull-based*: o chamador itera os resultados, o que permite parar cedo ou filtrar por método.

## Por quê

- **Memory/type safety sem confiar no emissor.** Se qualquer produtor de bytecode puder gerar sequências arbitrárias, a VM só é segura se verificar ou confiar. O .NET moderno optou por *confiar* em runtime (código nativo e `unsafe` existem) e deslocou a verificação para *ferramenta de build/teste*.
- **Pilha tipada = verificação em passada linear.** Como cada instrução tem efeito de pilha determinístico, uma única passada (com iteração para pontos de junção) basta, sem inferência complexa de tipos.
- **Autores de compiladores são o público.** O README diz explicitamente que o consumidor é quem emite IL (compiladores, profilers). Erros do emissor aparecem cedo, com offset e opcode.
- **Desacoplamento da CoreLib.** O verificador usa seu próprio `ILVerifyTypeSystemContext`, para poder verificar até a própria biblioteca base.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Verificação fora do runtime (ferramenta) | Startup rápido; nada de custo em JIT | Código não verificado pode rodar; segurança depende de confiança no emissor |
| Pilha com poucos tipos básicos | Verificador simples (`StackValueKind` ordenado) | Informação de tipo precisa ser reconstruída em cada ponto (subtipagem consulta o type system) |
| Verificar estrutura de EH explicitamente | Impede saltos para dentro de `try` etc. | Muitas regras específicas (dezenas de códigos em `VerifierError.cs`) |
| Rastrear `this` não inicializado | Impede observar objeto semi-construído | Regra especial só para ctors de classe, não generalizável sem análise de fluxo completa |
| Dependências explícitas na CLI | Reprodutível | Fricção: o usuário precisa passar todas as referências (`-r`) |

Limitação confirmada: a verificação cobre o que ECMA-335 define; ela **não** substitui análises de nível de fonte (nulidade, alcançabilidade etc.), que são tratadas nos outros arquivos desta série.

## Como aplicar na sua linguagem

1. **Defina um bytecode (ou IR) de pilha tipada, com número pequeno de "kinds" de pilha**, ordenados de modo que resultados de operações binárias sejam `max(kind)` — como o `StackValueKind` do ILVerify. Isso simplifica a regra de promoção numérica.
2. **Torne o verificador um componente independente da stdlib**: ele deve aceitar um `IResolver`/contexto de tipos abstrato para poder verificar a própria biblioteca padrão.
3. **Especifique regras de fluxo de controle**: pilha idêntica (compatível) em todo ponto de junção; proibir saltos para dentro de regiões protegidas; proibir queda do fim do método. Dê um código de erro a cada regra (enum estável + tabela de mensagens, como `VerifierError` + `Strings.resx`).
4. **Modele inicialização de objetos desde o início.** O caso `ThisUninit*` mostra que construtores precisam de regra específica; se sua linguagem permite acessar `this` antes da base ser construída, projete a análise de fluxo no verificador.
5. **Decida a política de confiança**: (a) verificar sempre ao carregar (seguro, mais lento), (b) verificar só em ferramenta (modelo .NET atual). Documente que código "unsafe" é uma categoria explícita.
6. **Teste com fixtures textuais** do bytecode: um arquivo por funcionalidade, métodos nomeados `_Valid` / `_Invalid_<Erro>`, para que o conjunto de testes sirva como especificação executável.
7. **Emita a posição (offset/opcode/slot) em cada erro** — o `VerifierError` reserva marcadores `E_OFFSET`, `E_OPCODE`, `E_STACK_SLOT`, `E_LOC`, `E_ARG`, `E_FOUND`, `E_EXPECTED` (comentados no arquivo) exatamente para isso.

## Apêndice: esqueleto de um verificador de pilha (derivado da estrutura do ILVerify)

O pseudocódigo abaixo é uma **síntese didática** da estrutura observada em `ILImporter.Verify.cs` (não é cópia do código):

```text
verificar(método):
    pilha := []                         # StackValue por slot
    fila  := [bloco_de_entrada]         # worklist de blocos básicos
    enquanto fila não vazia:
        b := fila.pop()
        carregar_estado_de_entrada(b)   # pilha, this_inicializado, ...
        para cada instrução i em b:
            efeito := tabela[i.opcode]  # quantos operandos, que tipos
            desempilhar e checar tipos  # CheckIsAssignable / CheckIsNumeric / ...
            empilhar resultado
            se i é salto/branch: propagar(pilha, alvo)
        se b "cai" no próximo: propagar(pilha, próximo)
        se b termina sem ret/br/throw: erro MethodFallthrough

propagar(pilha, alvo):
    se alvo ainda não tem estado: alvo.estado := cópia(pilha); fila.push(alvo)
    senão:
        se profundidade difere:  erro PathStackDepth
        senão para cada slot: se tipos incompatíveis: erro PathStackUnexpected
```

Detalhes confirmados que esse esqueleto representa: `InitialPass()` (linha ~315 de `ILImporter.Verify.cs`) prepara blocos; `basicBlock.IsThisInitialized` guarda o estado do `this`; erros de profundidade e de caminho existem no enum (`PathStackDepth`, `PathStackUnexpected`).

### Tabela de decisões de desenho para o seu verificador

| Pergunta | Resposta do ILVerify | O que decidir na sua linguagem |
|---|---|---|
| O verificador roda no carregamento? | Não: é ferramenta (`dotnet-ilverify`) | Verificar sempre / sob demanda / só em CI |
| Onde ficam as regras? | Código C# em `ILImporter.Verify.cs` + mensagens em `Strings.resx` | Tabelas declarativas por opcode vs. código |
| Como testar? | Fixtures `.il` com sufixos `_Valid`/`_Invalid_*` | Fixtures textuais do seu bytecode |
| Como tratar tipos gerados/genéricos? | Contexto de tipos próprio (`ILVerifyTypeSystemContext`, `InstantiatedGenericParameter.cs`) | Verificar sobre instanciação ou sobre definição genérica |
| Como reportar erros? | `VerificationResult` com `Code`, `Method`, `Type`, `Message`, `Args` | Estruturado (para IDE) + texto |

### Erros comuns de emissores (o que o verificador pega)

1. Esquecer de empilhar valor de retorno (`ReturnMissing`) ou deixar lixo na pilha (`ReturnEmpty`).
2. Saltar para dentro de um `try` (`BranchIntoTry`) ou sair de um `finally` com `br` (`BranchOutOfFinally`).
3. Usar `callvirt` em método estático (`CallVirtOnStatic`) ou em método de value type (`CallVirtOnValueType`).
4. Usar `this` antes de chamar o construtor-base (`ThisUninitStore`, `ThisUninitReturn`).
5. Escrever em campo `initonly` fora do construtor (`InitOnly`).
6. Chamar `ldftn`/`ldvirtftn` em construtor (`LdftnCtor`).

Cada um é um bom *teste de aceitação* para o seu próprio emissor.

## Referências

- `src/coreclr/tools/ILVerify/README.md`
- `src/coreclr/tools/ILVerification/Verifier.cs`
- `src/coreclr/tools/ILVerification/ILImporter.Verify.cs` (`ImportBinaryOperation`, `CheckIsAssignable`, `_trackObjCtorState`, `InitLocals`)
- `src/coreclr/tools/ILVerification/ILImporter.StackValue.cs`
- `src/coreclr/tools/ILVerification/ILImporter.ExceptionRegions.cs`
- `src/coreclr/tools/ILVerification/VerifierError.cs`
- `src/coreclr/tools/ILVerification/TypeVerifier.cs`, `AccessVerificationHelpers.cs`, `Strings.resx`
- `src/coreclr/tools/ILVerification.Tests/ILTests/` (fixtures `.il`)
- Padrão externo citado no README: ECMA-335 (não presente no repositório).
