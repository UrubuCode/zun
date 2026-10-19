# Shared generic instantiations e dicionários genéricos

> Fonte principal: `docs/design/coreclr/botr/shared-generics.md` (Fadi Hanna, 2019), cruzado com
> `src/coreclr/vm/genericdict.cpp/.h`, `generics.cpp` e `type-loader.md`.

## Mecanismo no .NET

### 1. Problema

`Method<string>` e `Method<object>` geram código quase idêntico. Exemplo do próprio documento:

```csharp
string Method<T>() { return typeof(List<T>).ToString(); }
```

Sem sharing, as duas versões diferem em uma única instrução: o `mov rcx, <TypeHandle de List<T>>`.
Com sharing, existe um corpo canônico (`Method<__Canon>`) que obtém o TypeHandle exato a partir do
**contexto genérico**:

```asm
mov rcx, generic context            ; MethodDesc de Method<string> ou Method<object>
mov rcx, [rcx + offset of InstantiatedMethodDesc::m_pPerInstInfo]  ; dicionário
mov rcx, [rcx + slot do TypeHandle de List<T>]
call ToString()
```

(`shared-generics.md`, "Canonical Codegen and Generic Dictionaries".)

### 2. Quando há sharing

"Esta feature é suportada apenas para instanciações sobre **tipos de referência**, porque todos
têm o mesmo tamanho/propriedades/layout. Para primitivos ou value types, o runtime gera corpos de
código separados para cada instanciação" (`shared-generics.md`, intro). No carregador:
instanciações com tipos de referência compartilham o mesmo `EEClass` e `MethodDesc`s; o
`MethodTable` canônico é criado antes da primeira instanciação com referência e guarda dados
quentes não específicos da instanciação, como slots não virtuais (`type-loader.md`, 2.2.2).

O argumento canônico é o tipo `System.__Canon`
(`src/coreclr/System.Private.CoreLib/src/System/__Canon.cs`).

### 3. Onde fica o dicionário

- **Métodos genéricos**: `InstantiatedMethodDesc::m_pPerInstInfo` aponta *diretamente* para o
  conteúdo do dicionário.
- **Tipos genéricos**: `MethodTable::m_pPerInstInfo` aponta para uma *tabela* de dicionários,
  porque tipos derivados herdam os dicionários dos tipos base. Exemplo do documento:

```csharp
class BaseClass<T> { }
class DerivedClass<U> : BaseClass<U> { }
class AnotherDerivedClass : DerivedClass<string> { }
```

| Tipo | `dictionaryTable` |
|---|---|
| `BaseClass<T>` | `[0]` = dicionário de BaseClass |
| `DerivedClass<U>` | `[0]` = dicionário de BaseClass, `[1]` = dicionário de DerivedClass |
| `AnotherDerivedClass` | `[0]` = dic. BaseClass, `[1]` = dic. DerivedClass (sem dicionário próprio, pois não é genérico) |

### 4. Slots

Um dicionário é um array de slots com informação específica da instanciação: type handles,
method handles, field handles, entry points...
- Os **primeiros N slots** (N = número de argumentos) são sempre os TypeHandles dos argumentos
  (otimização).
- Há um **slot de tamanho** (não usado pelo código genérico; parte da expansão dinâmica).
- Slots começam `NULL` e são populados *lazy* via `Dictionary::PopulateEntry(...)`.
- Slot nulo → helpers de runtime: `JIT_GenericHandleClass` (dicionário de tipo; usado por todos os
  métodos de instância de tipos genéricos) e `JIT_GenericHandleMethod` (dicionário de método;
  métodos genéricos e métodos estáticos não genéricos de tipos genéricos).

Exemplo do documento para `Method<string>`:

```
slot[0]: TypeHandle(string)
slot[1]: tamanho total do dicionário
slot[2]: TypeHandle(List<string>)
slot[3]: NULL
slot[4]: NULL
```

### 5. `DictionaryLayout`: quem decide qual slot é qual

O JIT precisa saber, em tempo de compilação do corpo canônico, *qual índice* contém o quê. Isso é o
`DictionaryLayout` (`genericdict.cpp`):
- É **associado à instanciação canônica**; `Method<object>` e `Method<string>` têm dicionários
  separados mas o *mesmo layout* (o de `Method<__Canon>`).
- A atribuição de slot é feita por `DictionaryLayout::FindToken(...)` com uma **assinatura**
  (descreve o tipo de valor do slot); busca linear: se achar assinatura igual devolve o índice;
  se achar entrada vazia, a preenche; senão precisa expandir.

### 6. Expansão dinâmica de dicionários

Historicamente os layouts eram uma lista encadeada de buckets de tamanho fixo (4 slots para métodos);
dicionários tinham tamanho fixo e só os "fast lookup slots" eram baratos, o resto ia por helper
lento. Com ReadyToRun + tiered compilation, os slots rápidos se esgotavam cedo e re-JIT acabava
usando helpers lentos. Solução: layouts viram arrays expansíveis.

Pontos de projeto importantes (todos do documento):
- Não dá para só redimensionar o layout (o JIT geraria offsets além do dicionário real).
- Não dá para realocar o dicionário no lugar: no caso de tipos ele faz parte do `MethodTable`; para
  métodos ainda pode estar em uso; e não pode haver dois MethodTable/MethodDesc para o mesmo tipo.
- Não dá para redimensionar só um dicionário: o código canônico serve a todas as instanciações.
- O runtime é multithread.

Implementação: layouts expandem em `DictionaryLayout::FindToken()`; dicionários de instâncias
expandem *sob demanda* via `Dictionary::GetTypeDictionaryWithSizeCheck()` /
`GetMethodDictionaryWithSizeCheck()`. A geração de código equivalente (JIT e R2R):

```c++
void* pDictionary = pMethodDesc->m_pPerInstInfo;
if (pDictionary[sizeOffset] <= requiredOffset || pDictionary[requiredOffset] == NULL)
    pResult = JIT_GenericHandleMethod(pMethodDesc, <signature>);
else
    pResult = pDictionary[requiredOffset];
```

A checagem de tamanho só é emitida para slots *além* do tamanho inicial alocado (para não
regredir desempenho). Dicionários antigos **não são liberados**; propagação do novo ponteiro para
tipos derivados é lazy em `JIT_GenericHandleWorker`.

### 7. Contraparte no compilador AOT

No type system gerenciado usado por crossgen2/ILC existem os arquivos `*.Canon.cs` em
`src/coreclr/tools/Common/TypeSystem/Canon/` (`InstantiatedType.Canon.cs`, `MethodDesc.Canon.cs`,
`GenericParameterDesc.Canon.cs`, `StandardCanonicalizationAlgorithm.cs` etc.) e `CanonicalFormKind`
(`Specific`, `Any`...) em `CanonTypes.cs`.

## Por quê

- **Reduzir memória e tempo de JIT** (objetivo declarado na introdução do documento).
- **Manter reificação sem pagar código duplicado**: o tipo exato continua acessível, só que via
  indireção.
- **Ser compatível com R2R/tiering**: motivou a expansão dinâmica.
- **Referências têm layout idêntico**: é a condição que torna o compartilhamento seguro.

## Trade-offs

- Cada acesso a metadado de tipo no código compartilhado custa uma ou duas indireções (carga do
  dicionário + carga do slot), e potencialmente um helper lento na primeira vez.
- Complexidade grande: layout vs dicionário, herança de dicionários, expansão concorrente, memória
  que nunca é devolvida.
- O contexto genérico precisa ser passado ao código compartilhado (parâmetro oculto, ou deduzido de
  `this`), afetando convenção de chamada e inlining.
- Value types ficam fora do sharing — mais código, porém mais rápido.
- Dificulta depuração (`__Canon` aparece em stack traces, como o próprio comentário de
  `__Canon.cs` admite).

## Como aplicar na sua linguagem

1. **Comece sem sharing** (especialize tudo, ou apague tudo). Só adicione sharing quando medir
   inchaço de código.
2. Se adicionar: defina um critério de "representação idêntica" (ex.: todos os argumentos são
   ponteiros) e uma forma canônica para o argumento.
3. Defina um **contexto genérico explícito** na convenção de chamada, e um dicionário por
   instanciação com `slot[0..N-1] = argumentos`.
4. **Layout por canônico, dicionário por instanciação**, com preenchimento lazy; reserve um slot de
   tamanho se quiser crescer depois.
5. Se tiver JIT/tiering, pense desde o início em **como o dicionário pode crescer** sem invalidar
   o código já gerado (a lição principal do documento).
6. Em AOT puro (sem JIT), prefira calcular dicionários estaticamente ("generic lookups" resolvidos
   na compilação) — a existência de `*.Canon.cs` mostra que o ILC faz isso.
7. Documente quais operações exigem o dicionário (cast a `T`, `new T()`, `typeof(T)`,
   chamada virtual/constrained sobre `T`).

## Referências

- `docs/design/coreclr/botr/shared-generics.md`
- `docs/design/coreclr/botr/type-loader.md` (2.2.2)
- `docs/design/coreclr/botr/method-descriptor.md` (`InstantiatedMethodDesc`)
- `src/coreclr/vm/genericdict.cpp`, `src/coreclr/vm/genericdict.h`
- `src/coreclr/vm/generics.cpp`, `src/coreclr/vm/generics.h`
- `src/coreclr/System.Private.CoreLib/src/System/__Canon.cs`
- `src/coreclr/tools/Common/TypeSystem/Canon/*`
