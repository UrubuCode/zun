# object como raiz unificada da hierarquia

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### System.Object é a raiz

`System.Object` (`src/libraries/System.Private.CoreLib/src/System/Object.cs`, parte compartilhada, e `src/coreclr/System.Private.CoreLib/src/System/Object.CoreCLR.cs`, parte específica do CoreCLR; a classe é `public partial class Object`) é a superclasse de todas as classes. O comentário do arquivo: "The Object is the root class for all object in the CLR System. Object is the super class for all other CLR objects and provide a set of methods and low level services to subclasses." (repo).

Membros (repo):

| Membro | Natureza |
|---|---|
| `Object()` | construtor, `[NonVersionable]` |
| `~Object()` (Finalize) | virtual; "This method's virtual slot number is hardcoded in runtimes. Do not add any virtual methods ahead of this." |
| `virtual string? ToString()` | padrão: `GetType().ToString()` |
| `virtual bool Equals(object?)` | padrão: identidade |
| `static bool Equals(object?, object?)` | helper estático |
| `static bool ReferenceEquals(object?, object?)` | identidade, `[NonVersionable]` |
| `virtual int GetHashCode()` | padrão: `RuntimeHelpers.GetHashCode(this)` |
| `Type GetType()` | não virtual, `[Intrinsic]`: lê o `MethodTable` do objeto (`RuntimeHelpers.GetMethodTable(this)` -> `RuntimeTypeHandle.GetRuntimeType`) |
| `protected internal object MemberwiseClone()` | `[Intrinsic]`: aloca clone e copia bytes brutos (com `Buffer.BulkMoveWithWriteBarrier` se há ponteiros GC) |

`GetType` e `MemberwiseClone` não são virtuais: o runtime garante que o tipo reportado é sempre o tipo real e que a cópia rasa tem comportamento fixo.

### Como o runtime representa a raiz

- Todo objeto tem como único campo obrigatório o ponteiro para `MethodTable` no offset 0, mais um `ObjHeader` em offset negativo (`src/coreclr/vm/object.h`, comentário em `class Object`: "The only fields mandated by all objects are a pointer to the MethodTable at offset 0 [and] a pointer to a ObjHeader at a negative offset") (repo). `OBJECT_SIZE` = tamanho de um ponteiro; `OBJECT_BASESIZE` = header + ponteiro (`object.h` ~89-90); `MIN_OBJECT_SIZE` define o menor objeto alocável.
- O runtime guarda um handle global para o tipo raiz: `g_pObjectClass = CoreLibBinder::GetClass(CLASS__OBJECT)` (`src/coreclr/vm/appdomain.cpp` ~959). É usado, por exemplo, como tipo de elemento de `object[]` (`g_pPredefinedArrayTypes[ELEMENT_TYPE_OBJECT]`, ~998) e na tabela de arrays (`array.cpp` ~232) (repo).
- Os slots virtuais de `Object` ficam no começo da vtable de todo tipo (arquivo 02); o comentário em `methodtable.h` sobre chunks de vtable menciona o tamanho inicial 4 "matching the number of virtuals in System.Object" (repo).

### Value types e boxing

`System.ValueType` é uma classe abstrata que deriva de `Object` (`src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`: `public abstract partial class ValueType`), e `Enum : ValueType` (`src/libraries/System.Private.CoreLib/src/System/Enum.cs`) (repo). Structs e enums derivam implicitamente de `ValueType`.

Mas um valor "solto" (local, campo, elemento de array) *não tem* cabeçalho nem `MethodTable` pointer: é só os bytes. Para tratar como `object`, o runtime faz *boxing*: aloca um objeto no heap cujo `MethodTable` é o do struct e copia os dados (`MethodTable::Box(void*)` e `FastBox`, `methodtable.h` ~2780; `IsValueType()` ~2849) (repo). `unbox` é a operação inversa com verificação de tipo. É isso que unifica o sistema de tipos sem forçar que todo inteiro seja objeto.

Interfaces também são tratadas como "coisas atribuíveis a object": `MethodTable::IsInterface()` (categoria `enum_flag_Category_Interface`); em C# toda referência para interface converte implicitamente para `object` (C#/ECMA).

### Consequências na biblioteca

- Contêineres não genéricos (`object[]`, `ArrayList`) e APIs como `string.Format(string, params object[])`, `Console.WriteLine(object)` dependem da raiz comum.
- `Nullable<T>`, `Enum`, delegates etc. participam da mesma hierarquia (categorias de `MethodTable`: `enum_flag_Category_Nullable`, etc.).
- Genéricos reificados reduzem a dependência de `object` para coleções (`List<T>`), mantendo-o para casos dinâmicos.

## Por quê

1. **Tipo universal para APIs e ferramentas**: depuração, reflexão, serialização, locks (`lock(obj)` usa o header: arquivo 09), hashing, GC e finalização podem operar sobre qualquer coisa que seja um objeto.
2. **Contrato mínimo garantido**: todo objeto sabe seu tipo (`GetType`), ser impresso, comparar e hashear; o dispatch virtual desses métodos tem índices fixos (o comentário "slot number is hardcoded").
3. **Unificar valores e referências com custo sob demanda**: structs são baratos (sem header), mas podem virar objeto por boxing quando necessário (inferência a partir do desenho: `Box` existe como operação explícita).
4. **GC/runtime precisam de um único formato de objeto**: `MethodTable*` + header + dados. O `ObjHeader` serve a vários propósitos (hash, thin lock, sync block), o que só funciona porque todo objeto o tem.
5. **Heap homogêneo**: um `object[]` pode conter qualquer coisa (referência a objetos de qualquer tipo, ou valores boxed).

## Trade-offs

- **Custo de espaço por objeto**: header + ponteiro de `MethodTable` (mínimo: `MIN_OBJECT_SIZE`). Em 64 bits são 16 bytes antes de qualquer campo (OBJHEADER_SIZE tem uma versão com padding de 4 bytes em 64 bits: `object.h` linhas 84-86, repo).
- **Boxing**: alocações e perda de localidade quando valores são tratados como `object`/interfaces. Muito esforço de design do .NET (genéricos, `IEquatable<T>`, `constrained.`) existe para evitá-lo.
- **Superfície obrigatória**: todo tipo herda `ToString/Equals/GetHashCode/Finalize/GetType/MemberwiseClone`, mesmo quando não fazem sentido (ex.: `Equals` para um tipo não comparável; o `Finalize` vazio, que o comentário `CA1821` suprime).
- **Slots hardcoded** em `Object` tornam a raiz imutável na prática: adicionar virtual antes quebra o runtime.
- **`MemberwiseClone` protegido, mas ativo para todos**: clonagem rasa é fácil de usar errado.
- **Tipos sem raiz** (ponteiros, `ref` e `ref struct`, tipos de função) ficam fora da unificação: `ref struct` não pode ser boxed. O próprio repo mostra a pressão no sistema de tipos com `allows ref struct` em `EventHandler`/`IEquatable<T>` (arquivos 08 e 09).
- **Dinâmica fraca em tempo de compilação**: `object` encoraja cast e checagens em runtime.

## Como aplicar na sua linguagem

1. **Decida se há uma raiz única.** Prós: API universal, GC simples, contêineres heterogêneos. Contras: custo por objeto, boxing, superfície obrigatória. Alternativa: sem raiz + traits/protocolos universais derivados (`Eq`, `Hash`, `Display` opcionais).
2. **Se tiver raiz, minimize a superfície.** Considere só: `type_of`, `equals`, `hash`, `to_string`. Mova `finalize`, `clone`, e `lock` para capacidades opt-in. O .NET lamenta ter `Finalize` e monitor em todo objeto.
3. **Fixe os slots da raiz** no início da vtable e documente que são ABI (modelo `Object.cs`).
4. **Objeto = [ponteiro de tipo][header opcional][campos]**. Mantenha o ponteiro de tipo na posição 0 e um header de tamanho palavra com bits multiplexados (hash/lock/flags) com promoção lazy para uma estrutura lateral (como `SyncBlock`).
5. **Tipos de valor**: se os terá, escolha: (a) sem header, com boxing explícito/implícito (modelo .NET); (b) todo valor é objeto (estilo Ruby/Python), simples mas caro; (c) valores sempre unboxed e sem conversão para a raiz. O modelo .NET oferece o melhor desempenho para numérico, ao custo de boxing e de uma "classe abstrata" `ValueType` que existe só para ser a raiz formal.
6. **Interfaces/traits** devem ser atribuíveis à raiz? Defina a conversão (no .NET é implícita e tem custo se for valor).
7. **GetType/typeof não virtual.** Garanta que o tipo dinâmico é confiável.
8. **Planeje `ref`-like/estruturas de pilha** que não podem virar objeto: defina uma restrição explícita (`allows ref struct`) em genéricos desde o começo.
9. **Defina equality/hash/to_string defaults** (arquivo 09) e avalie se o default deve ser identidade ou erro de compilação.
10. **Reflexão e metadados**: com uma raiz, `GetType()` leva ao descritor de tipo (`MethodTable` -> `RuntimeType`); defina o equivalente.

## Resumo dos fatos confirmados

- `Object` é `partial class` com partes comuns e específicas do CoreCLR.
- `GetType` é `[Intrinsic]` e lê o `MethodTable` do objeto.
- `MemberwiseClone` aloca um clone e copia bytes (com write barrier quando há referências GC).
- O runtime guarda `g_pObjectClass` como tipo fundamental.
- Layout mínimo de objeto: `ObjHeader` (offset negativo) + ponteiro de `MethodTable` (offset 0).
- `ValueType` e `Enum` derivam de `Object`, mas valores soltos não têm header; boxing cria o objeto.
- Slot do finalizador é fixo em `Object`.

## Camadas do objeto na memória (CoreCLR)

1. `[ObjHeader]` (negativo): hash, thin lock, índice de sync block, bits de flag.
2. `[MethodTable*]` (offset 0): tipo exato.
3. `[campos]`: pai primeiro, depois os do filho.
4. Strings e arrays têm campos de tamanho/contagem adicionais (`ARRAYBASE_SIZE` em `object.h`).

## Alternativas à raiz única (conhecimento geral)

| Modelo | Exemplo | Observação |
|---|---|---|
| Raiz única para tudo | Smalltalk, Ruby | mais simples, custo por valor |
| Raiz para referências, valores à parte | .NET, Java | boxing necessário |
| Sem raiz | C++, Rust | polimorfismo por traits/templates |
| Raiz opcional por protocolo | Swift (`Any`), Go (`any`) | menos custo, menos garantia |

## Perguntas de design

- Todo valor pode virar `Any`? Quando e como (boxing, tagged pointers)?
- Quais operações são universais? Mantenha o mínimo (tipo, igualdade, hash, texto).
- O cabeçalho de objeto é obrigatório? Quantos bytes você aceita pagar?
- `finalize` e `lock` em todo objeto são desejáveis?
- Como representar tipos que não são objetos (ponteiros, refs de pilha)?
- Como a reflexão chega do objeto ao descritor de tipo?
- A raiz é parte da ABI? Quais slots ficam fixos?

## Glossário

- `ObjHeader`: palavra de header anterior ao ponteiro de tipo, multiplexada entre hash, lock e sync block.
- `MethodTable`: descritor de tipo em runtime, apontado pelo offset 0 do objeto.
- `SyncBlock`: estrutura lateral alocada sob demanda para lock e hash quando o header não basta.
- Boxing: criar um objeto no heap a partir de um valor.
- Unboxing: acessar o valor dentro de um objeto boxed, com verificação de tipo.
- `ValueType`: classe abstrata base de structs e enums.
- Tipo fundamental: tipo que o runtime precisa conhecer cedo, como `System.Object`.
- `CoreLibBinder`: componente do VM que localiza tipos e métodos fundamentais em CoreLib.

## Protótipo mínimo de objeto e raiz (pseudocódigo, proposta minha)

```
struct Header { bits: u64 }              // hash | lock | ponteiro p/ estrutura lateral
struct Obj    { header: Header, type: *TypeInfo /* campos... */ }

class Object {                           // raiz
    virtual fn to_string() -> str { type_of(self).name }
    virtual fn equals(o: Object?) -> bool { self is o }
    virtual fn hash() -> i32 { identity_hash(self) }
    final   fn type_of() -> TypeInfo { self.type }
}

fn box<T: ValueType>(v: T) -> Object {
    let o = alloc(TypeInfo_of<T>)
    copy_bytes(o.data, &v, size_of<T>)
    return o
}
```

Notas:

- O ponteiro de tipo no offset 0 permite `type_of` e a vtable sem indireções extras.
- O header fica antes do ponteiro de tipo, como no CoreCLR (`ObjHeader` em offset negativo).
- `box` espelha `MethodTable::Box`: aloca com o tipo do struct e copia os dados.
- `unbox` precisa verificar `o.type == TypeInfo_of<T>`.
- Slots de `to_string`, `equals`, `hash` ficam nas primeiras posições da vtable e são ABI.
- `type_of` é final: o tipo dinâmico não pode ser mentido.
- Se há finalizadores, mantenha-os como capacidade opcional em vez de membro da raiz.
- Se há locks por objeto, use o header com promoção lazy para uma estrutura lateral.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Object.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/Object.CoreCLR.cs` (`GetType`, `MemberwiseClone`).
- `src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`; `src/libraries/System.Private.CoreLib/src/System/Enum.cs`.
- `src/coreclr/vm/object.h` (`class Object`, `OBJECT_SIZE`, `OBJHEADER_SIZE`, `MIN_OBJECT_SIZE`).
- `src/coreclr/vm/appdomain.cpp` (`g_pObjectClass`, ~959; array de `object`, ~998).
- `src/coreclr/vm/methodtable.h` (`Box`, `FastBox`, `IsValueType`, `IsInterface`).
