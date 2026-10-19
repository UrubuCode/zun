# IL para generics (instanciação, dicionários genéricos)

## Mecanismo no .NET

Generics são um recurso **de primeira classe do CLR**: o IL e a metadata preservam parâmetros de tipo (reificação), em vez de apagá-los (type erasure). `List<int>` e `List<string>` são tipos distintos em runtime. Isso exige representar genéricos na metadata, nas assinaturas, no IL e no código gerado.

### Na metadata (ECMA-335 Partição II §22)

- **`GenericParam`** (token `mdtGenericParam = 0x2a000000`): uma linha por parâmetro de tipo/método (número, flags, nome, dono — `TypeDef` ou `MethodDef`).
- **`GenericParamConstraint`** (`mdtGenericParamConstraint = 0x2c000000`): uma linha por restrição (`where T : IFoo`).
- **`MethodSpec`** (`mdtMethodSpec = 0x2b000000`): instanciação de método genérico (`Foo<int>` aplicado a método) — contém o método e uma assinatura de instanciação.
- **`TypeSpec`** (`mdtTypeSpec = 0x1b000000`): tipo construído; o blob contém `GENERICINST` (arquivo 04).
- Flags de parâmetro (`CorGenericParamAttr`, `src/coreclr/inc/corhdr.h`): `gpNonVariant=0`, `gpCovariant=1`, `gpContravariant=2`, `gpReferenceTypeConstraint=0x4`, `gpNotNullableValueTypeConstraint=0x8`, `gpDefaultConstructorConstraint=0x10`, `gpAllowByRefLike=0x20`.

### Nas assinaturas

- `ELEMENT_TYPE_VAR n` (0x13): n-ésimo parâmetro do **tipo** envolvente.
- `ELEMENT_TYPE_MVAR n` (0x1E): n-ésimo parâmetro do **método**.
- `ELEMENT_TYPE_GENERICINST <CLASS|VALUETYPE> <tipo> <n> <args...>` (0x15): instancia um tipo genérico.
- Convenção `IMAGE_CEE_CS_CALLCONV_GENERIC (0x10)` para método genérico (a contagem de parâmetros de tipo precede a de parâmetros) e `GENERICINST (0xa)` para blobs de `MethodSpec`.

Exemplo, `static T Id<T>(T x)`: o método tem assinatura `10 01 01 1E 00 1E 00` (GENERIC, 1 tipo, 1 param, retorno `MVAR 0`, param `MVAR 0`). Um chamador `Id<int>(5)` faz `call <MethodSpec>` cujo blob é `0A 01 08` (GENERICINST, 1 arg, `I4`).

### No IL

O IL é o **mesmo** para todas as instanciações; instruções relevantes são `constrained.` (chamar membro sobre valor de tipo variável sem box prévio), `box`/`unbox.any` (que se comportam de forma diferente para `T` valor ou referência), `ldtoken`, `newobj`, `initobj` e `sizeof` usando `!0`/`!!0`. O addendum do repo trata de interações: `constrained.` (Augments, "III.2.1 constrained."), "ByRefLike types in generics" (`docs/design/features/byreflike-generics.md`): se `T` é ByRefLike, `box` lança `InvalidProgramException`; `ldsfld/stsfld`, `newarr/ldelem/stelem` lançam `TypeLoadException`; `constrained. callvirt` é válido (com `NotSupportedException` no call-site se o alvo resolve para `object`/DIM).

### Estratégia de execução: código compartilhado × especializado

`docs/design/coreclr/botr/shared-generics.md` (autor: Fadi Hanna, 2019) e `ilc-architecture.md` descrevem:

1. **Instanciações sobre tipos-valor** (`List<int>`, `List<KeyValuePair<int,int>>`): **código especializado** por instanciação — tamanho/layout/propriedades diferem. "This feature is currently only supported for instantiations over reference types".
2. **Instanciações sobre tipos-referência** (`List<string>`, `List<object>`): todas compartilham **um único corpo canônico**, `List<__Canon>`, pois referências têm o mesmo tamanho/GC layout. `__Canon` é um tipo especial; o ILC o define em `src/coreclr/tools/Common/TypeSystem/Canon/CanonTypes.cs` (`CanonType`, com `CanonicalFormKind.Specific`; há também `UniversalCanonType`, "a struct of indeterminate size and GC layout", `CanonicalFormKind.Universal`).
3. Formas mistas: `List<KeyValuePair<int,string>>` canonicaliza para `List<KeyValuePair<int,__Canon>>`.

### O problema: o código canônico não sabe o tipo exato

No exemplo do doc:

```csharp
string Method<T>() { return typeof(List<T>).ToString(); }
```

o código para `Method<string>` e `Method<object>` seria idêntico exceto pelo handle do tipo `List<T>`. O corpo canônico lê o handle de um **dicionário genérico**:

```asm
mov rcx, generic context                                  ; MethodDesc de Method<string> (ou MethodTable do this)
mov rcx, [rcx + offset InstantiatedMethodDesc::m_pPerInstInfo]   ; dicionário
mov rcx, [rcx + slot do handle de List<T>]
call ToString()
```

**Contexto genérico** (argumento oculto): para métodos genéricos, o `MethodDesc` instanciado; para métodos de instância em tipo genérico, derivado do `this` (`MethodTable`); para estáticos em tipos genéricos, parâmetro oculto com o `MethodTable`. (Do doc: o "generic context" é um parâmetro oculto — "Shared generic code has a hidden parameter that represents the type parameters", `readytorun-overview.md`.)

### Dicionários genéricos

- Um **dicionário** é um array de slots **por instanciação** (`Method<string>` tem o seu; `Method<object>` tem outro), com informações específicas: type handles, method handles, field handles, entry points.
- Os primeiros N slots são os type handles dos N argumentos de tipo; seguem os slots de informação derivada. Exemplo do doc para `Method<string>`: `slot[0]: TypeHandle(string)`, `slot[1]: tamanho total do dicionário`, `slot[2]: TypeHandle(List<string>)`, `slot[3..]: NULL`.
- Os slots são **preenchidos preguiçosamente**: se `NULL`, o código chama `JIT_GenericHandleClass` (tipos) ou `JIT_GenericHandleMethod` (métodos/estáticos em tipo genérico) que resolve via `Dictionary::PopulateEntry` e grava no slot.
- **Em tipos**, `MethodTable::m_pPerInstInfo` aponta para uma tabela de dicionários, um por nível de herança — porque classes derivadas herdam os dicionários dos pais. No exemplo: `BaseClass<T>`, `DerivedClass<U> : BaseClass<U>`, `AnotherDerivedClass : DerivedClass<string>` — a última não tem dicionário próprio, mas herda os ponteiros dos dois anteriores. **Em métodos**, `m_pPerInstInfo` do `InstantiatedMethodDesc` aponta direto para os dados.

### `DictionaryLayout`

Compartilhado por todas as instanciações compatíveis (associado à forma canônica `Method<__Canon>`): diz **qual slot** guarda **qual assinatura**. O JIT chama `DictionaryLayout::FindToken(...)` ao precisar de um lookup; o algoritmo (do doc):

```
slot = 0
para cada entry no layout:
    se entry.signature != NULL:
        se entry.signature == inputSignature: retorna slot
    senão:
        entry.signature = inputSignature
        retorna slot
    slot++
```

### Expansão dinâmica de dicionários

Antigamente o layout era lista encadeada de buckets de tamanho fixo (4 slots para métodos); estourar significava usar helpers lentos. Com R2R + tiered compilation, os slots "rápidos" acabavam sendo consumidos por código R2R e o rejit ficava sem slots; por isso foi criada a **expansão dinâmica**: layout vira array expansível. Pontos delicados descritos no doc:

- não dá para redimensionar só o layout (o JIT emitiria acessos além do tamanho dos dicionários existentes);
- não dá para realocar `MethodTable`/`MethodDesc`;
- não dá para expandir só um dicionário (o código canônico é compartilhado por todas as instanciações);
- há multithreading.

A solução: expande-se o layout quando acabam os slots (`DictionaryLayout::FindToken`) e os dicionários **lazy**, quando alguém lê um slot além do tamanho (`Dictionary::GetTypeDictionaryWithSizeCheck`/`GetMethodDictionaryWithSizeCheck` em `src/coreclr/vm/genericdict.cpp`). Código gerado, equivalente a:

```c++
if (pDictionary[sizeOffset] <= requiredOffset || pDictionary[requiredOffset] == NULL)
    pResult = JIT_GenericHandleMethod(pMethodDesc, <signature>);
else
    pResult = pDictionary[requiredOffset];
```

A checagem de tamanho só é emitida para slots além do número **inicial** de slots. Dicionários antigos não são desalocados (complexidade em ambiente multi-thread sem benefício útil). Em tipos, a propagação do novo ponteiro às classes derivadas é lazy, no `JIT_GenericHandleWorker`.

### Em R2R

Genéricos são fonte de fixups específicos (`readytorun-format.md`): `READYTORUN_FIXUP_ThisObjDictionaryLookup (0x07)`, `TypeDictionaryLookup (0x08)`, `MethodDictionaryLookup (0x09)`; método com `READYTORUN_METHOD_SIG_MethodInstantiation (0x04)`, `InstantiatingStub (0x02)` ("does not take hidden dictionary generic argument") e `UnboxingStub`. A seção `InstanceMethodEntryPoints` (hashtable) guarda entrypoints de instanciações. As regras de bolha (`crossgen2-compilation-structure-enhancements.md`): instanciação só entra na imagem se definição e argumentos estão na bolha, com exceção de `__Canon`. Seções v9: `MethodIsGenericMap`, `TypeGenericInfoMap`.

### Em Native AOT

`ilc-architecture.md`, "Compiling generics": o ILC compila sempre a forma **canônica** (`List<__Canon>.Add`) e gera um **dicionário** por instanciação (`List<string>`). Tipos auxiliares:

- **`RuntimeDeterminedType`**: par (canon, parâmetro genérico de origem), p.ex. `T` de `Foo<T>`, representado como `List<T_System.__Canon>`; necessário porque a forma canônica perde a identidade de parâmetros (`T` e `U` viram `__Canon`) e a não instanciada não tem informação concreta (`sizeof(T)`).
- **Shadow method nodes** (`ShadowConcreteMethodNode`, `ShadowNonConcreteMethodNode`): representam instanciações parciais/totais só para rastrear dependências; instanciam dependências "abstratas" (MethodTable de `List<T>`) em concretas (MethodTable de `List<string>`) e alimentam os dicionários. Dicionários não podem reportar dependências diretamente porque só se sabe o conjunto completo depois de descobrir todos os métodos que o usam.
- **Generic virtual methods (GVM)**: precisam de método *e* dicionário resolvidos em runtime pelo tipo do receptor + instanciação. O ILC usa `GVMDependenciesNode` com dependências **dinâmicas** (observa quais tipos são "construídos" e inclui overrides); em runtime há uma **tabela de GVM**.

## Por quê

- **Desempenho sem boxing**: instanciação sobre valores gera código especializado, sem box e com inlining.
- **Menos código**: compartilhar `__Canon` evita explosão de instanciações sobre tipos-referência ("reduce the amount of code the runtime generates ... reduce memory footprint, and the amount of time we spend jitting").
- **Reificação**: `typeof(T)`, `new T[]`, `is T` funcionam em runtime; é o que o dicionário fornece.
- **Compatibilidade com tiering/R2R**: dicionários expansíveis permitem R2R e rejit coexistirem sem perder lookups rápidos.

## Trade-offs

| Escolha | Benefício | Custo |
|---|---|---|
| Reificação (não apagar) | Tipos exatos em runtime, sem box para valores | Runtime precisa de carregador de tipos genérico, dicionários, `MethodSpec`/`TypeSpec` |
| Especializar para valores | Melhor código | Explosão de código (cada `T` de valor gera corpo próprio); em AOT precisa saber todas as instanciações (ou fallback JIT) |
| Compartilhar para referências | Menos código/JIT | Lookup indireto via dicionário em operações que dependem de `T`; precisa de argumento oculto |
| Dicionário por instanciação, layout por forma canônica | Slots consistentes entre instanciações | Expansão dinâmica complexa e multithread; memória não liberada |
| Slots preenchidos lazy | Startup barato | Primeiro acesso paga helper |
| GVM | Flexibilidade | Dependências dinâmicas no compilador AOT e tabela em runtime; são "expensive" |
| ByRefLike como argumento | Habilita `Action<Span<T>>` | Várias instruções passam a falhar em instanciações (box, static, array) |

## Como aplicar na sua linguagem

1. **Escolha entre apagamento, reificação ou monomorfização.** O .NET faz híbrido: monomorfiza valores, compartilha referências. Se seu runtime tem GC e objetos uniformes por referência, esse híbrido é uma boa base.
2. **Represente variáveis de tipo por posição** (`VAR n`/`MVAR n`) e separe o espaço de tipo e de método. Tenha uma tabela de parâmetros (com variância e restrições) e uma de instanciações de método (`MethodSpec`).
3. **Defina a forma canônica** (um tipo sentinela como `__Canon`) e a regra: "todas as instâncias com argumentos-referência compartilham o mesmo código". Considere uma forma "universal" para casos de tamanho desconhecido (como `UniversalCanon`).
4. **Passe um contexto genérico oculto** ao código compartilhado e guarde nele (ou nos objetos) um ponteiro para o dicionário. Fixe o layout: primeiros slots = argumentos de tipo.
5. **Preencha dicionários lazy e permita expansão**: planeje o crescimento desde o começo (array expansível + checagem de tamanho só além do inicial), em vez de bucket fixo.
6. **No compilador AOT**: modele `RuntimeDeterminedType`-like (par canônico + parâmetro de origem) e nós "sombra" para instanciar dependências; trate métodos virtuais genéricos com dependência condicional/dinâmica.
7. **Defina cedo as regras de variância e restrições** em flags (`CorGenericParamAttr`), e resolva `constrained.` para evitar boxing em chamadas via interface em tipos-valor.
8. **Decida como reflexão e `typeof(T)` funcionam** em código compartilhado: ou via dicionário, ou proibindo.
9. **Versione**: no R2R, genéricos cruzando bolhas exigem fixups de dicionário; defina limites (só compile instanciações cujas partes estão na sua bolha).
10. **Teste o caso de herança de dicionários** (tipo não genérico derivando de genérico fechado) — é fonte clássica de bugs.

## Referências

- `docs/design/coreclr/botr/shared-generics.md` — dicionários, layouts, expansão dinâmica.
- `docs/design/coreclr/botr/ilc-architecture.md` — "Compiling generics" (canonicalização, runtime-determined, GVM, shadow nodes).
- `docs/design/features/byreflike-generics.md`; `docs/design/specs/Ecma-335-Augments.md` (constrained, static virtual, "ByRefLike types in generics").
- `src/coreclr/inc/corhdr.h` — `ELEMENT_TYPE_VAR/MVAR/GENERICINST`, `CorGenericParamAttr`, `mdtGenericParam`, `mdtMethodSpec`.
- `src/coreclr/vm/genericdict.cpp` — `Dictionary`, `DictionaryLayout`.
- `src/coreclr/tools/Common/TypeSystem/Canon/CanonTypes.cs` — `CanonType`, `UniversalCanonType`, `CanonicalFormKind`.
- `docs/design/coreclr/botr/readytorun-format.md`, `docs/design/features/crossgen2-compilation-structure-enhancements.md` — genéricos em R2R.
- ECMA-335 Partição II §22.20 (GenericParam), §22.29 (MethodSpec), §22.21 (GenericParamConstraint), §II.9 (genéricos); Partição I §8.7 (variância).

## Apêndice A — Mini-exemplo de ponta a ponta

```csharp
class Box<T> { public T Value; public string Describe() => typeof(List<T>).Name; }
var a = new Box<string>(); var b = new Box<object>(); var c = new Box<int>();
```

- Metadata: um `TypeDef` `Box`1` com uma linha em `GenericParam` (T). Os usos em `new Box<string>()` geram `TypeSpec`s com `GENERICINST CLASS <TypeDef Box`1> 1 STRING`; `Box<int>` usa `... 1 I4`.
- Execução (JIT ou AOT):
  - `Box<string>` e `Box<object>` compartilham o corpo canônico de `Describe` (`Box<__Canon>.Describe`), cada um com seu dicionário (`slot[0] = TypeHandle(string)` / `TypeHandle(object)`), e o slot do handle de `List<T>` é preenchido lazy na primeira chamada.
  - `Box<int>` recebe **corpo próprio** (`Box<int>.Describe`) porque `T` é tipo-valor: o JIT/ILC conhece `List<int>` estaticamente, sem dicionário.
- Em R2R: o corpo canônico entra na imagem se `Box` está na bolha; a instanciação `Box<int>` pertence à bolha de `Box`, assim como `MyGeneric<int>` no exemplo do doc (que contrasta com `MyGeneric<DateTime>`, só na bolha de `MyGeneric` se esta compartilhar bolha com `System.Private.CoreLib`).

## Apêndice B — Referência rápida de campos e termos

| Termo | Onde aparece | Significado |
|---|---|---|
| `__Canon` | VM e ILC | tipo-sentinela: "qualquer referência" |
| `__UniversalCanon` | ILC (`CanonTypes.cs`) | struct de tamanho/GC layout indeterminados (forma "universal") |
| `m_pPerInstInfo` | `MethodTable`, `InstantiatedMethodDesc` | ponteiro para o(s) dicionário(s) |
| `DictionaryLayout` | `genericdict.cpp` | associação slot → assinatura, por forma canônica |
| `JIT_GenericHandleClass/Method` | VM | helpers que preenchem slots NULL |
| `READYTORUN_FIXUP_*DictionaryLookup` | R2R | lookups genéricos em código pré-compilado |
| `RuntimeDeterminedType` | ILC | par (tipo canônico, parâmetro de origem) |
| `ShadowConcrete/NonConcreteMethodNode` | ILC | nós de dependência por instanciação, sem código |
| `GVMDependenciesNode` | ILC | dependências dinâmicas para métodos virtuais genéricos |

## Apêndice C — Perguntas de projeto sobre genéricos para uma nova linguagem

1. **Apagar ou reificar?** Se reificar, você precisa de dicionários (ou outra forma de passar tipos) e de `typeof(T)` em runtime.
2. **Quais tipos compartilham código?** O .NET usa a regra "todo tipo-referência é igual para fins de código" — simples, mas exclui structs. Se sua linguagem tem só referências, o compartilhamento é total.
3. **Variância**: `gpCovariant`/`gpContravariant` existem na metadata (flags de `GenericParam`) e afetam verificação de atribuição, não geração de código.
4. **Restrições**: `gpReferenceTypeConstraint`, `gpNotNullableValueTypeConstraint`, `gpDefaultConstructorConstraint`, `gpAllowByRefLike`. As duas primeiras são cruciais para escolher entre compartilhar e especializar.
5. **Interface com despacho virtual**: métodos virtuais *genéricos* exigem pesquisa em duas dimensões (tipo do receptor × instanciação) — reserve tabela de GVM ou use estratégia de especialização global (no AOT).
6. **Reflexão**: `MakeGenericType` em runtime cria instâncias não previstas em AOT; o ILC lida com templates em `System.Private.TypeLoader`. Decida se sua linguagem as suporta.
