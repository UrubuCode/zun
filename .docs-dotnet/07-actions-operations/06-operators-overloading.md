# Operadores e operator overloading

## Mecanismo no .NET

No .NET **não existem operadores na CIL para tipos definidos pelo usuário**. Um operador sobrecarregado é um **método estático com nome especial** (`op_Addition`, `op_Equality`, `op_Implicit`...) marcado `specialname` nos metadados. O compilador da linguagem (C#, F#, VB) traduz `a + b` em `call T.op_Addition(a, b)` quando os tipos não são primitivos. Para primitivos o compilador emite os opcodes IL (`add`, `ceq`, `clt`...) diretamente.

### Evidências no repositório

1. **Biblioteca**: `Half` declara operadores como métodos nomeados. O JIT os lista como intrinsics: `NI_System_Half_op_Addition`, `op_Decrement`, `op_Division`, `op_Equality`, `op_Explicit`, `op_GreaterThan`, `op_GreaterThanOrEqual`, `op_Increment`, `op_Inequality`, `op_LessThan`, `op_LessThanOrEqual`, `op_Multiply`, `op_Subtraction` (`src/coreclr/jit/namedintrinsiclist.h`, linhas ~28-40). Isto mostra que o nome do método É o contrato do operador.
2. **Tipos do sistema**: `NI_System_Type_op_Equality` e `op_Inequality` (mesmo arquivo, ~linha 131). `MulticastDelegate` define `operator ==`/`!=` em C# (`src/libraries/System.Private.CoreLib/src/System/MulticastDelegate.cs`) com `[MethodImpl(AggressiveInlining)]` e um comentário: "Can't call the == operator as it will call object==" — ilustrando que dentro do operador o `==` precisa evitar recursão (use `ReferenceEquals`).
3. **Expression trees** reconhecem operadores definidos pelo usuário: `src/libraries/System.Linq.Expressions/src/System/Linq/Expressions/BinaryExpression.cs` tem `GetUserDefinedBinaryOperator(ExpressionType binaryType, string name, Expression left, Expression right, bool liftToNull)` (~linha 602) e `GetUserDefinedBinaryOperatorOrThrow`. A busca é por **nome** (`"op_Addition"`) na assinatura dos tipos dos operandos, e também tenta o tipo *não-anulável* (`nnLeftType`) para *lifting* de `Nullable<T>`.
4. **dynamic**: `Microsoft.CSharp.RuntimeBinder` (`CSharpBinaryOperationBinder.cs`, `CSharpUnaryOperationBinder.cs`) resolve operadores em runtime usando as mesmas regras do compilador (ver arquivo 09).

### Generic math (operadores em interfaces)

O .NET moderno declara operadores como membros **abstratos estáticos de interface** (ex. `IAdditionOperators<TSelf,TOther,TResult>` em `src/libraries/System.Private.CoreLib/src/System/Numerics/IAdditionOperators.cs`; `static abstract TResult operator +(TSelf, TOther)`). Isso permite código genérico `T Sum<T>(T a, T b) where T : IAdditionOperators<T,T,T>` sem boxing nem delegates. (Os arquivos em `System/Numerics/` existem no repositório; o formato exato do membro vem do conhecimento da linguagem C#.)

### Conversões

`op_Implicit` e `op_Explicit` implementam conversões definidas pelo usuário. `NI_System_Half_op_Explicit` aparece na lista de intrinsics. Em Expression Trees, `Expression.Convert(expr, type, method)` aceita o método de conversão.

### Checked vs unchecked

Operadores aritméticos de primitivos têm formas IL com e sem overflow (`add` / `add.ovf`). Para tipos de usuário o C# 11 introduz `op_CheckedAddition` etc. (conhecimento geral; veja os `op_Checked*` nas interfaces de `System.Numerics`).


### Tabela de nomes convencionais (parte confirmada pelo repositório)

| Operador | Nome do método | Confirmado em |
|---|---|---|
| `+` | `op_Addition` | `NI_System_Half_op_Addition` |
| `-` | `op_Subtraction` | `NI_System_Half_op_Subtraction` |
| `*` | `op_Multiply` | `NI_System_Half_op_Multiply` |
| `/` | `op_Division` | `NI_System_Half_op_Division` |
| `==` | `op_Equality` | `NI_System_Half_op_Equality`, `NI_System_Type_op_Equality` |
| `!=` | `op_Inequality` | `NI_System_Half_op_Inequality`, `NI_System_Type_op_Inequality` |
| `<`, `<=`, `>`, `>=` | `op_LessThan`, `op_LessThanOrEqual`, `op_GreaterThan`, `op_GreaterThanOrEqual` | `NI_System_Half_op_*` |
| `++`, `--` | `op_Increment`, `op_Decrement` | `NI_System_Half_op_Increment`, `op_Decrement` |
| cast explícito | `op_Explicit` | `NI_System_Half_op_Explicit` |

Os demais (`op_Implicit`, `op_UnaryNegation`, `op_BitwiseAnd`, `op_LeftShift`, `op_Modulus`, `op_True`/`op_False`) são convenção ECMA-335/C# e não foram contados um a um neste repositório.

### Como `Expression.Add(a, b)` escolhe o operador (fluxo observável em `BinaryExpression.cs`)

```text
Expression.Add(left, right)
  tipos aritméticos primitivos -> nó Add intrínseco
  senão GetUserDefinedBinaryOperatorOrThrow(Add, "op_Addition", left, right, liftToNull)
     GetUserDefinedBinaryOperator(binaryType, left.Type, right.Type, name)   // busca por nome
     se não achou e os tipos são Nullable<T>: repete com os tipos não anuláveis (lifting)
     se achou: monta BinaryExpression com o MethodInfo do operador
```

Os nomes `GetUserDefinedBinaryOperator` e `GetUserDefinedBinaryOperatorOrThrow` e o uso de `nnLeftType`/`nnRightType` estão no arquivo; a árvore de decisão acima é um resumo do que aparece nas linhas ~602-690.

### O mesmo operador, três caminhos de execução

1. Estático (compilador C#): `call Half::op_Addition`; o JIT pode torná-lo instrução.
2. Árvore de expressão: nó `BinaryExpression` com `Method` = `op_Addition`; ao compilar, `call` no IL gerado.
3. `dynamic`: `CSharpBinaryOperationBinder` resolve em runtime; o resultado vira regra de call site (arquivo 09).

### Operadores e genéricos

Interfaces de generic math ficam em `src/libraries/System.Private.CoreLib/src/System/Numerics/` (existe `IAdditionOperators.cs` e família). A ideia: `T` restrito a uma dessas interfaces usa `a + b` resolvido por especialização, sem delegate nem boxing, e o JIT pode inlinar o operador concreto.

### Pegadinhas de projeto

- `==` sobrecarregado e `ReferenceEquals`: `MulticastDelegate.cs` evita usar `==` dentro do próprio operador.
- Operadores devem ser estáticos e públicos e conter um parâmetro do tipo declarante (regra do C#; não verificável aqui).
- Um método chamado `op_X` sem `specialname` não é reconhecido como operador pelo compilador (convenção de metadados).
- Operadores de conversão não participam de despacho virtual; para polimorfismo use método de instância.
- Atribuição composta (`+=`) é derivada do operador binário pelo compilador, não é método separado.


### Glossário

| Termo | Significado |
|---|---|
| `specialname` | marca de metadados que torna o método um operador/acessor reconhecido |
| Lifting | aplicar operador de `T` a `Nullable<T>` propagando ausência |
| Static abstract member | membro estático exigido por interface, resolvido por especialização |
| Generic math | conjunto de interfaces numéricas baseadas em static abstract |

### Testes sugeridos

1. Sobrecarga de `==` com receptores `null` em ambos os lados.
2. Operador mais específico entre candidatos de dois tipos diferentes.
3. Lifting com valores ausentes.
4. Função genérica que soma `T` para `int`, `double` e um tipo definido pelo usuário.
5. Comparar o custo de `a + b` em tipo primitivo, em `Half` e em tipo de usuário.

### Pontos em aberto para a sua linguagem

- Permitir criar operadores novos (símbolos) ou só sobrecarregar os existentes?
- Precedência e associatividade são fixas ou declaráveis?
- Operadores são métodos ordinários (podem ser passados como função) ou construtos especiais?
- Resolução considera conversões implícitas nos dois operandos ou só no primeiro?
- Há distinção entre operador checado e não checado?


### Operadores unários, de conversão e de igualdade: regras práticas

| Categoria | Comportamento observado ou convencional |
|---|---|
| Unário (`-x`, `!x`, `~x`) | `op_UnaryNegation`, `op_LogicalNot`, `op_OnesComplement` (convenção) |
| Conversão implícita | `op_Implicit`, não pode lançar por convenção |
| Conversão explícita | `op_Explicit`, pode lançar; presente em `Half` como intrinsic |
| Igualdade | `op_Equality`/`op_Inequality` e `Equals`/`GetHashCode` devem concordar |
| Comparação | quatro operadores relacionais mais `IComparable` |
| Compostos (`+=`) | derivados do binário pelo compilador |

### Exemplo de operador intrínseco no JIT

`Half.op_Addition` está no enum `NamedIntrinsic`; assim, quando o hardware tem aritmética de meia precisão (ou o JIT escolhe uma sequência conveniente), a chamada ao operador vira IR direto. Sem isso, executa o corpo gerenciado de `Half.cs`. Este é o argumento mais forte do .NET para operadores como métodos comuns: o caminho de otimização já existe para métodos.

### Operadores no JIT para primitivos

Para `int`, `long`, `float`, `double` o compilador de linguagem emite diretamente `add`, `sub`, `mul`, `div`, `ceq`, `clt`, `cgt` (ECMA-335). Estes nunca passam por `op_*`. A tabela de opcodes do JIT está em `src/coreclr/jit/gentreeopsdef.h` (operações da árvore, por exemplo `GT_ADD`).

## Por quê

- **Sem novo opcode**: o modelo de execução permanece "só chamadas"; operadores são açúcar sintático sobre métodos estáticos. Qualquer linguagem .NET pode chamar `op_Addition` mesmo sem suporte a sintaxe de operador.
- **O JIT otimiza por inlining e intrinsics**: `Half.op_Addition` vira instrução quando o hardware suporta.
- **Reflection e ferramentas descobrem operadores** por nome convencional, o que viabiliza Expression Trees e `dynamic`.
- **Interoperabilidade entre linguagens** sem exigir um operador "universal".

## Trade-offs

- Convenção por **nome de método** é frágil: um erro de grafia desativa o operador silenciosamente; a semântica de lifting para Nullable é implementada de novo em cada compilador.
- Pares obrigatórios (`==` e `!=`; `<`/`>`; `true`/`false`) são regras do compilador, não do runtime.
- Operadores estáticos não têm despacho virtual: `a == b` com `a: Base` e `b: Base` chama o operador de `Base` (daí o `Equals`/`GetHashCode` adicional e o aviso `CS0660/CS0661` suprimido no `MulticastDelegate.cs`).
- Sobrecarga excessiva prejudica leitura e pode ocultar custo (alocação em `+`).
- Operadores em interfaces (generic math) deslocam complexidade para o runtime: abstract static members exigem resolução no type loader e geram especializações genéricas.

## Como aplicar na sua linguagem

1. Decida: operadores são **sintaxe sobre métodos** (modelo .NET) ou **construtos primitivos** do IR? A primeira opção é a mais simples de implementar e a mais extensível.
2. Fixe uma tabela de nomes estável (`op_Addition`...) e marque os métodos como especiais nos metadados para que ferramentas os identifiquem.
3. Especifique regras de seleção (overload resolution) para operadores binários: candidatos de ambos os operandos e de interfaces; defina o lifting para tipos opcionais.
4. Ofereça operadores em **traits/interfaces** (equivalente a static abstract) se quiser código genérico numérico sem custo.
5. Assegure que operadores sobre primitivos virem instruções diretas e que operadores de tipos de biblioteca (Half, vetores) possam ser intrínsecos.
6. Proíba ou avise sobre redefinição de `==` sem `hash`/`equals` coerentes.
7. Permita `checked`/`unchecked` por operador.

## Referências

- `src/coreclr/jit/namedintrinsiclist.h` (`NI_System_Half_op_*`, `NI_System_Type_op_*`)
- `src/libraries/System.Private.CoreLib/src/System/MulticastDelegate.cs`
- `src/libraries/System.Linq.Expressions/src/System/Linq/Expressions/BinaryExpression.cs` (`GetUserDefinedBinaryOperator`)
- `src/libraries/Microsoft.CSharp/src/Microsoft/CSharp/RuntimeBinder/CSharpBinaryOperationBinder.cs`
- `src/libraries/System.Private.CoreLib/src/System/Numerics/` (interfaces de generic math)
- `src/libraries/System.Private.CoreLib/src/System/Half.cs`
