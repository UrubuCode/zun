# Invocação via reflection (MethodInfo.Invoke)

## Mecanismo no .NET

`MethodInfo.Invoke(object, object[])` chama um método descoberto em runtime. Em CoreCLR a implementação é híbrida: primeiro por um caminho nativo lento, depois — após o método ter sido invocado — por um **stub IL gerado em `DynamicMethod`** e cacheado.

### Camadas

| Camada | Arquivo |
|---|---|
| Objetos de reflexão (`RuntimeMethodInfo`, `RuntimeConstructorInfo`) | `src/coreclr/System.Private.CoreLib/src/System/Reflection/RuntimeMethodInfo.CoreCLR.cs`, `RuntimeConstructorInfo.CoreCLR.cs` |
| Invocador por método | `src/libraries/System.Private.CoreLib/src/System/Reflection/MethodBaseInvoker.cs` (classe `MethodBaseInvoker`), `MethodBaseInvoker.Constructor.cs`, `MethodInvoker.cs`, `ConstructorInvoker.cs` |
| Parte CoreCLR do invoker | `.../Reflection/MethodBaseInvoker.CoreCLR.cs`, `IntrinsicInvokeHelper.cs`, `InstanceCalliHelper.cs` |
| Escolha de estratégia | `.../Reflection/MethodInvokerCommon.cs` (`DetermineStrategy_*`) |
| Emissão do stub | `.../Reflection/InvokerEmitUtil.cs` |
| Conversão de argumentos | `.../Reflection/InvokeUtils.cs` |
| Núcleo nativo | `src/coreclr/vm/reflectioninvocation.cpp` (1476 linhas) |

### Estratégias

`MethodBase.InvokerStrategy` (`MethodBase.cs` ~linha 169) é um enum de flags com três famílias, cada uma com `HasBeenInvoked_*` e `StrategyDetermined_*`:

- `ObjSpanArgs` — argumentos como `Span<object?>`;
- `Obj4Args` — até 4 argumentos como parâmetros `object` diretos (sem array);
- `RefArgs` — argumentos por ponteiro (`IntPtr* refArguments`) para parâmetros `ref`/ponteiro.

Delegates do stub (`InvokerEmitUtil.cs`): `InvokeFunc_RefArgs(object? obj, IntPtr* refArguments)`, `InvokeFunc_ObjSpanArgs(object? obj, Span<object?> arguments)`, `InvokeFunc_Obj4Args(object? obj, object? arg1..arg4)`.

### Política "lento primeiro, rápido depois"

Em `MethodInvokerCommon.DetermineStrategy_RefArgs` (e variantes):

```csharp
if ((strategy & InvokerStrategy.HasBeenInvoked_RefArgs) == 0) {
    // Start with the slow path unless a forced strategy has already been selected.
    strategy |= InvokerStrategy.HasBeenInvoked_RefArgs;
} else {
    if (RuntimeFeature.IsDynamicCodeCompiled)
        invokeFunc_RefArgs = CreateInvokeDelegate_RefArgs(method, backwardsCompat);
    strategy |= InvokerStrategy.StrategyDetermined_RefArgs;
}
```

Isto é: a **primeira** chamada usa o caminho lento (nativo/interpretado, sem emitir IL); só a **segunda** emite o `DynamicMethod`. Se `IsDynamicCodeCompiled` for falso (AOT/interpretado), nunca emite e permanece no caminho lento.

### O stub emitido

`InvokerEmitUtil.CreateInvokeDelegate_Obj4Args` cria `new DynamicMethod("InvokeStub_" + tipo + "." + nome, typeof(object), {object x5}, typeof(object).Module, skipVisibility: true)`; o comentário diz "If changed, update native stack walking code that also uses this prefix to ignore reflection frames". O IL resultante carrega `this` (se `hasThis`), faz unbox/cast dos argumentos, chama o método e **faz box do retorno** (ou `newobj` para construtores concretos). `skipVisibility: true` permite chamar membros privados. O delegate resultante: `dm.CreateDelegate(typeof(InvokeFunc_Obj4Args), target: null)`.

### Pontos adicionais

- Frames `InvokeStub_` são ignorados no stack walk (por prefixo) para esconder o stub de stack traces.
- Exceções do método chamado são embrulhadas em `TargetInvocationException` (comportamento padrão da API; `backwardsCompat` controla o embrulho em algumas APIs).
- Métodos intrínsecos do JIT invocados via reflexão executam o corpo gerenciado (`vectors-and-intrinsics.md`: "Indirect invocation through reflection, a delegate, or a function pointer executes the intrinsic method body").
- Tipos genéricos abertos não são invocáveis (`Debug.Assert(!method.ContainsGenericParameters)` no emitter).
- `Delegate.DynamicInvoke` e `Activator.CreateInstance` seguem trilha análoga.


### Fluxo da chamada `mi.Invoke(obj, args)`

```text
RuntimeMethodInfo.Invoke(obj, ..., args)
  valida alvo e argumentos; coerção de tipos (InvokeUtils / Binder)
  -> MethodBaseInvoker (criado e cacheado no MethodInfo)
     escolhe aridade: poucos args -> Obj4Args; senão ObjSpanArgs; com ref/ponteiro -> RefArgs
     DetermineStrategy_*:
        1a chamada: caminho lento (nativo, reflectioninvocation.cpp); marca HasBeenInvoked_*
        2a chamada: se IsDynamicCodeCompiled -> emite InvokeStub_* (DynamicMethod)
     executa o delegate emitido ou o caminho lento
  exceção do alvo -> TargetInvocationException
```

A sequência combina `MethodBaseInvoker.cs`, `MethodInvokerCommon.cs` e `InvokerEmitUtil.cs`, que foram lidos; o nome exato dos métodos de validação em `RuntimeMethodInfo` não foi confirmado.

### Três formas de pagar menos que `MethodInfo.Invoke`

| Técnica | Custo por chamada | Observações |
|---|---|---|
| `MethodInvoker` | baixo; evita revalidação | classe em `MethodInvoker.cs` |
| `CreateDelegate` | quase de delegate normal | exige assinatura conhecida |
| `Expression` compilada | idem delegate | custo inicial de `Compile` |

### Casos especiais citados no código

- Construtores: `emitNew = method is RuntimeConstructorInfo && !method.DeclaringType!.IsAbstract`; o stub usa `newobj`.
- Instância vs estático: `hasThis = !emitNew && !method.IsStatic`; o primeiro parâmetro do stub é `object? obj` e é ignorado em estáticos (o comentário diz que tratar o DynamicMethod como instância é "slightly faster").
- `Debug.Assert(!method.ContainsGenericParameters)`: genéricos abertos não chegam ao emitter.
- Intrinsics de CoreLib invocados por reflexão executam o corpo gerenciado (arquivo 05).
- `InstanceCalliHelper.cs` e `instancecalli.cpp`: infraestrutura de `calli` de instância.

### Efeitos em stack walking e GC

O comentário de `InvokeStubPrefix` diz que o stack walker nativo usa o prefixo `InvokeStub_` para ignorar frames de reflexão. O `DynamicMethod` é gerenciado por `dynamicmethod.cpp` no VM.

### Quando NÃO usar reflexão para chamar

- Caminho quente (use delegate, `calli` ou geração de código).
- Código AOT/trimmed (sem metadados).
- Assinatura conhecida em compile-time.

### Matriz de custo (qualitativa)

| Operação | Alocações típicas | Observação |
|---|---|---|
| Invoke (1a vez) | `object[]`, boxes | caminho lento |
| Invoke (a partir da 2a) | boxes de valor/retorno | stub emitido |
| `Obj4Args` | sem array | até 4 parâmetros |
| `RefArgs` | ponteiros para args | para ref/out/ponteiros |
| `CreateDelegate` + chamada | nenhuma | melhor para repetição |

### Pontos a especificar na sua linguagem

1. Representação de valores "caixa" (boxing) para argumentos e retorno.
2. Regras de conversão automática (widening, `Type.Missing`, valores padrão).
3. Visibilidade: reflexão ignora ou respeita encapsulamento?
4. Exceções: embrulhar ou propagar.
5. Genéricos: exigir instanciação fechada antes de invocar.

## Por quê

- **Chamada tardia por nome/token** é pré-requisito para serialização, DI, frameworks de teste, plug-ins.
- **Estratégia em dois estágios** evita pagar o custo de gerar IL (compilar + JIT) para métodos que são invocados uma só vez.
- **Diferentes aridades**: `Obj4Args` evita alocar `object[]` para o caso comum (até 4 args).
- **Stub IL** reduz o custo por chamada de ~marshalling genérico para chamada quase direta.

## Trade-offs

- Mesmo no melhor caso, há box dos retornos e valores, checagem de tipos e cópia de argumentos — mais lento que delegate (`CreateDelegate`) ou `calli`.
- Primeira chamada paga conversão genérica; segunda paga emissão IL + JIT.
- Memória: `DynamicMethod` por método reflexivo.
- Não funciona sem geração de código; AOT limita reflexão (trimming remove metadados).
- Visibilidade ignorada (`skipVisibility`) = quebra de encapsulamento por design.
- Semântica de exceção embrulhada complica depuração.

## Como aplicar na sua linguagem

1. Se você tem reflexão, desenhe **uma API de invoker pré-vinculado** (equivalente a `MethodInvoker`): resolve método uma vez, reutiliza muitas vezes — o repositório tem `MethodInvoker.cs` e `ConstructorInvoker.cs` exatamente para isso.
2. Use **estratégia adaptativa**: interpretado/lento na 1ª chamada, compilado após reuso.
3. Evite `object[]`: ofereça sobrecargas para aridades pequenas e um caminho por referência para `ref`/ponteiros.
4. Defina a política de exceções (embrulho vs. propagação direta).
5. Esconda frames de infraestrutura nas stack traces com convenção de nome (`InvokeStub_`).
6. Se pretende AOT, permita que o compilador gere a ponte estática para métodos referenciados conhecidos e exija anotações para os dinâmicos.
7. Considere integrar com delegates: converter `MethodInfo` em delegate tipado costuma ser o caminho recomendado para chamada repetida.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Reflection/MethodBaseInvoker.cs`, `MethodInvokerCommon.cs`, `InvokerEmitUtil.cs`, `MethodBase.cs`
- `src/coreclr/System.Private.CoreLib/src/System/Reflection/MethodBaseInvoker.CoreCLR.cs`, `IntrinsicInvokeHelper.cs`, `InstanceCalliHelper.cs`
- `src/coreclr/vm/reflectioninvocation.cpp`
- `src/coreclr/vm/dynamicmethod.cpp`
- `docs/design/coreclr/botr/vectors-and-intrinsics.md`
