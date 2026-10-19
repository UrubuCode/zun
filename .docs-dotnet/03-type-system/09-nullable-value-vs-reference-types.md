# Nullable value types vs nullable reference types

## Mecanismo no .NET

São dois mecanismos completamente diferentes com o mesmo nome:

| | `T?` onde `T : struct` | `T?` onde `T : class` (NRT) |
|---|---|---|
| Existe em runtime? | Sim: `System.Nullable<T>` (tipo real) | Não: anotação de compilador |
| Representação | struct `{ bool hasValue; T value; }` | Mesma referência de sempre |
| Onde é checado | Runtime + compilador | Só compilador (warnings) |
| Metadado | Tipo genérico instanciado | Atributos `[Nullable]`, `[NullableContext]` |

### 1. `Nullable<T>` — tipo real

`src/libraries/System.Private.CoreLib/src/System/Nullable.cs`:

```csharp
// Do NOT add any interfaces to Nullable!
[NonVersionable] // This only applies to field layout
public partial struct Nullable<T> where T : struct
{
    private readonly bool hasValue; // Do not rename (binary serialization)
    internal T value;               // Do not rename ...
    ...
    public readonly T Value { get { if (!hasValue) ThrowHelper...NoValue(); return value; } }
    public readonly T GetValueOrDefault() => value;
    public readonly T GetValueOrDefault(T defaultValue) => hasValue ? value : defaultValue;
```

Pontos confirmados:
- Constraint `where T : struct`; a constraint exclui `Nullable<>` (ver `04`), logo não há
  `Nullable<Nullable<T>>`.
- O comentário "Do NOT add any interfaces to Nullable!" é relevante: o runtime trata o box
  especialmente; interfaces do tipo subjacente são o que o objeto boxed expõe, não as do wrapper.
- `[NonVersionable]` fixa o layout (JIT e R2R dependem dele).
- Layout: `hasValue` primeiro, `value` depois; o MethodTable guarda os offsets em
  `m_encodedNullableUnboxData` ("offset do campo value e tamanho do campo value",
  `src/coreclr/vm/methodtable.h` ~4040), com accessors em ~3087-3114
  (`GetNullableValueAddrOffset` etc.).

### 2. Categoria própria no MethodTable

`enum_flag_Category_Nullable = 0x00050000`, "sub-category of ValueType" (`methodtable.h` ~3822).
`IsNullable()` é um teste de categoria (~2867).

### 3. Boxing especial

- Box: `hasValue == false` → `null`; senão boxa o `T` (`Nullable::Box`, `src/coreclr/vm/object.cpp`
  ~1357; versão gerenciada `CastHelpers.Box_Nullable`).
- `CEEInfo::getTypeForBox` mapeia `Nullable<T>` → `T` (`jitinterface.cpp` ~6193).
- Unbox para `Nullable<T>` (`CastHelpers.Unbox_Nullable`, ~624): `null` → zera o struct (cleared,
  `hasValue=false`); objeto cujo tipo é `T` → seta `hasValue=true` e copia `value`
  (com write barrier se `ContainsGCPointers`); outro tipo → `InvalidCastException`.
- Consequência: `(object)(int?)null == null`, e `isinst`/`unbox.any` tratam o box de `int`
  como um `int?`.

### 4. Nullable Reference Types (NRT): puro metadado

Não há alteração no MethodTable nem no GC. O compilador emite atributos em
`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/`:
- `NullableAttribute` (`byte[] NullableFlags`) — estado de nulidade por posição de tipo
  (0 = oblivious, 1 = não-nulo, 2 = anulável; convenção do C#, conhecimento geral);
- `NullableContextAttribute` — default para um escopo (reduz tamanho do metadado);
- `NullablePublicOnlyAttribute`.
Os atributos são "Reserved for use by a compiler for tracking metadata. This attribute should not
be used by developers in source code" (comentário em `NullableAttribute.cs`). O runtime
**ignora** esses atributos. Assim, `string?` e `string` são o mesmo tipo em runtime;
`List<string?>` e `List<string>` são o mesmo tipo; não há variância ou overload por NRT.

### 5. Interação com genéricos

- Código genérico com `T?` sem constraint tem semântica diferente conforme `T`: se `T` é
  referência, anotação; se `T` é struct, `Nullable<T>`. Por isso C# exige `where T : struct` ou
  `where T : class` para que `T?` tenha um significado único (conhecimento geral).
- `Nullable<T>` é uma instanciação sobre struct → **não é compartilhada** (código próprio por `T`).
- O argumento `default(T)` em genéricos: para struct é zero-bits (inclui `hasValue=false`).

### 6. Defaults e inicialização

`default(Nullable<T>)` = bits zero = `hasValue=false`. `Unbox_Nullable(null)` limpa memória com
`ClearWithoutReferences/ClearWithReferences`, ou seja, a "ausência" é o padrão zero — propriedade
que o runtime explora (arrays zerados já são arrays de `null`s).

## Por quê

- **Value types não têm "null" natural**: representar ausência exige um bit extra + o valor.
  `Nullable<T>` é o menor design genérico para isso (1 bool + padding).
- **Equivalência com null no box** preserva o contrato "um `object` null é ausência".
- **NRT como anotação**: reificar nulidade de referências quebraria bilhões de linhas de código e
  a identidade de tipos entre assemblies já compilados; por isso foi adicionado como camada de
  análise estática (inferência razoável a partir de o runtime ignorar os atributos).
- **Constraint `struct` exclui `Nullable`**: mantém `T??` inexistente e simplifica o box.

## Trade-offs

- `Nullable<T>`: custo de memória/alinhamento (ex.: `int?` ocupa 8 bytes), código especial em
  carregador, JIT e helpers, e armadilhas de box (box de `int?` não é `Nullable<int>`).
- Não é um tipo "soma" geral (só struct).
- NRT: sem garantia em runtime (`null!`, reflexão, código legado); exige disciplina e gera
  warnings; atributos aumentam metadado; "oblivious" cria zona cinza.
- Duas semânticas para `T?` aumentam a complexidade do compilador e a dificuldade de ensinar.
- Interoperação: APIs genéricas precisam de `where T : struct`/`class` para ter significado claro.

## Como aplicar na sua linguagem

1. **Escolha uma única história de ausência.** Opções:
   - tipo opção/soma geral (`Option<T>`/`Maybe`), com `None` como valor — uniforme para ref e valor;
   - `null` universal + análise estática (NRT);
   - ambos (como .NET), pagando a dualidade.
2. **Se criar `Nullable<T>`-like**, fixe o layout (`NonVersionable`), garanta que zero-bits = ausência
   e defina o box (`None`→null ou objeto) conscientemente. O .NET precisou de ao menos: categoria
   no MethodTable, offsets do campo `value`, helpers de box/unbox e mapeamento no JIT.
3. **Se criar NRT-like**, guarde nulidade em metadado compacto com *contexto padrão* por escopo
   (a razão de existir `NullableContext`).
4. **Não permita aninhar** (`T??`), via constraint.
5. **Decida a política de erro**: warnings (NRT) vs erro de compilação vs checagem em runtime.
6. **Ofereça operadores de ergonomia** (`?.`, `??`, `!`) como açúcar sobre o modelo escolhido.
7. **Torne o "zero" válido**: arrays e structs zerados devem ser estados válidos.

### Exemplo comparativo

```csharp
int? a = null;               // Nullable<int>: struct {hasValue=false, value=0}; 8 bytes
string? s = null;            // string normal; [Nullable(2)] só em metadado
object oa = a;               // null  (box especial)
object ob = (int?)3;         // box de int (não de Nullable<int>)
Console.WriteLine(ob.GetType());   // System.Int32
```

### Checklist de decisões

- [ ] Referências podem ser nulas por padrão? Há análise estática de nulidade?
- [ ] Ausência para tipos de valor: `Option` genérico ou tipo especial do runtime?
- [ ] Zero-bits é um estado válido (default)?
- [ ] Box do opcional: `null` ou wrapper?
- [ ] Onde a nulidade é registrada (tipo ou metadado)? Qual o contexto padrão?

### Observação final

Em código genérico compartilhado (`T` = referência, forma `__Canon`), `T?` de NRT não muda
nada em runtime; já `Nullable<T>` sempre vive em instanciações de valor, com código próprio.
Isso mostra por que as duas features não se unificam sem uma regra explícita na linguagem.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Nullable.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/NullableAttribute.cs`
- `.../CompilerServices/NullableContextAttribute.cs`, `.../NullablePublicOnlyAttribute.cs`
- `src/coreclr/vm/methodtable.h` (`enum_flag_Category_Nullable`, `IsNullable`, `m_encodedNullableUnboxData`)
- `src/coreclr/vm/object.cpp` (`Nullable::Box`, `UnBox`)
- `src/coreclr/vm/jitinterface.cpp` (`getTypeForBox`)
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs` (`Box_Nullable`, `Unbox_Nullable`)
- `src/coreclr/vm/typedesc.cpp` (constraint `struct` exclui `Nullable`, ~1358)
