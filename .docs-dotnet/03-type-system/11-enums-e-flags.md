# Enums e flags

## Mecanismo no .NET

### 1. Enum = value type com um tipo integral subjacente

Em metadado, um enum é um tipo que estende `System.Enum` (que estende `System.ValueType`) com um
**campo de instância** especial `value__` do tipo integral subjacente e campos estáticos literais
para as constantes (ECMA-335, conhecimento geral). No runtime, `System.Enum` é uma classe abstrata
(`src/libraries/System.Private.CoreLib/src/System/Enum.cs`: `public abstract partial class Enum`)
e os enums concretos recebem a categoria `Primitive` no `MethodTable`:

```
enum_flag_Category_Primitive     = 0x00060000  // sub-category of ValueType; used for enum types
enum_flag_Category_TruePrimitive = 0x00070000  // Int32, etc.
```

(`src/coreclr/vm/methodtable.h` ~3824-3825). O comentário de `IsTruePrimitive()` (~2817): "returns
false for enum types"; existe outro predicado que também retorna true para enums (~2820). Ou seja,
enums são tratados como primitivos pelo layout/JIT (representação = inteiro), mas distintos para
reflexão.

Tamanho, alinhamento e passagem em registrador são os do tipo subjacente; não há header.

### 2. Tipos subjacentes suportados

`Enum.cs` (~62-70) despacha por tipo subjacente: `sbyte/byte`, `short/ushort`, `int/uint`,
`long/ulong`, `nint/nuint`, `float`, `double`, `char` (e `bool` por extensão, não verificado). O
runtime armazena os valores em um tipo "de armazenamento" sem sinal (`TStorage`: "either the same as
TUnderlying or its unsigned counterpart", ~1488).

### 3. Informação reflexiva cacheada: `EnumInfo<TStorage>`

`src/libraries/System.Private.CoreLib/src/System/Enum.EnumInfo.cs`:

```csharp
internal sealed partial class EnumInfo<TStorage> where TStorage : struct, INumber<TStorage>
{
    public readonly bool HasFlagsAttribute;
    public readonly bool ValuesAreSequentialFromZero;
    public readonly TStorage[] Values;
    public readonly string[] Names;
    // valores e nomes ordenados por valor (Array.Sort se não estiverem ordenados)
}
```

Isto é: `ToString`, `GetName`, `IsDefined`, `Parse` usam arrays paralelos ordenados (busca
binária) e um atalho quando os valores são sequenciais a partir de zero (índice direto, ex. linha
~195 `ulong.CreateTruncating(value) < (ulong)names.Length`). O cache vive no `RuntimeType`
(`Enum.CoreCLR.cs`).

### 4. `[Flags]`

`FlagsAttribute` não muda a representação: só ativa um **modo de formatação/parse**
(`EnumInfo.HasFlagsAttribute`). Com Flags, `ToString` chama `FormatFlagNames`: decompõe o valor em
nomes de bits separados por vírgula; se sobra bit sem nome, devolve o número (`result ?? ...ToString()`
em ~1473-1477). Sem Flags, só casa valor exato. O formato `'f'` força a decomposição.

Operações de bit são do compilador (`|`, `&`, `^`) sobre o inteiro subjacente. O runtime não
verifica que as constantes sejam potências de dois.

### 5. `HasFlag`

```csharp
[Intrinsic]
public bool HasFlag(Enum flag)
{
    ArgumentNullException.ThrowIfNull(flag);
    if (GetType() != flag.GetType() && !GetType().IsEquivalentTo(flag.GetType()))
        throw new ArgumentException(...);
    ref byte pThisValue = ref this.GetRawData(); ...
    switch (InternalGetCorElementType()) { case ELEMENT_TYPE_I1: ... }
```

(`Enum.cs` ~398). É `[Intrinsic]` porque o JIT o expande para `(x & f) == f` quando o tipo é
conhecido, eliminando o boxing: antes dessa otimização `HasFlag(Enum)` exigia dois boxes (receptor e
argumento são `Enum`). Para o caso genérico o código acessa os bytes crus do objeto boxed.

### 6. Enums em genéricos e APIs modernas

- `Enum.GetNames<TEnum>()`/`GetValues<TEnum>()` usam `where TEnum : struct, Enum` (`Enum.cs`
  ~219, 290). A constraint `Enum` (e `delegate`, `unmanaged`) é possível porque o metadado e o
  `TypeVarTypeDesc` sabem tratar `System.Enum` como constraint de base (`ConstrainedAsObjRefHelper`
  exclui explicitamente `g_pEnumClass` como constraint de referência — `typedesc.cpp` ~1050).
- Formatação interna usa **generic math**: `where TUnderlying : struct, INumber<TUnderlying>,
  IBitwiseOperators<...>, IMinMaxValue<...>` (`Enum.cs` ~1482). Ver `04`.
- Unbox entre enum e seu tipo subjacente é permitido pelo runtime: `Unbox_Helper` aceita tipos
  primitivos com o mesmo `CorElementType` (`CastHelpers.cs` ~666) — logo `(int)(object)MyEnum.A`
  funciona.

### 7. Igualdade, hash, conversões

Enums herdam de `ValueType`/`Enum`; comparações usam `CompareTo` por tipo subjacente. A conversão
de/para inteiro é cast de representação (zero custo no código compilado). Valores fora do conjunto
nomeado são **válidos** (`(MyEnum)99`): o runtime não impõe pertencimento.

## Por quê

- **Eficiência**: enum = inteiro; sem alocação, comparável em registrador, bom para flags de bit e
  para interop com C.
- **Metadados ricos**: nomes e valores existem em runtime (reflexão, `ToString`, `Parse`).
- **Flags como convenção + atributo**: a mesma representação serve a "enum de escolhas" e a
  "conjunto de bits"; o atributo informa só a formatação.
- **Escolha do tipo subjacente**: compatibilidade com formatos binários/ABI.

## Trade-offs

- **Sem exaustividade nem segurança de valores**: qualquer inteiro é válido; `switch` não é
  verificado como exaustivo pelo runtime. (Pattern-matching/warnings são do compilador.)
- **Flags é só convenção**: nada impede valores sobrepostos; `HasFlag` e `ToString` têm semânticas
  sutis (`None=0`).
- **Reflexão/ToString é custosa**: precisa de cache por tipo, busca binária, alocações de strings.
- **Boxing**: quando enums viram `object`/`Enum` (`HasFlag` antes de ser intrínseco; coleções
  não genéricas).
- **Evolução**: adicionar membro altera valores observáveis; enums de assemblies diferentes
  sofrem de problemas de versionamento (constantes são inlined pelo compilador — conhecimento geral).
- **Sem comportamento associado**: ao contrário de enums de Java/Rust, não carregam campos ou métodos.

## Como aplicar na sua linguagem

1. **Decida se enum é "inteiro nomeado" ou "tipo soma".** O .NET escolhe inteiro nomeado; se você
   quer exaustividade e dados associados, use tipos soma (e mantenha um enum simples só para flags/
   interop).
2. **Se for inteiro nomeado**: tipo subjacente configurável, categoria própria no descritor de tipo
   (tratado como primitivo para layout/ABI), campo `value__`-like.
3. **Flags como tipo distinto** (ex.: `flags enum` ou `bitset`) em vez de atributo permite
   verificar potências de dois e operações (`has`, `|`) em compile-time.
4. **Guarde nomes/valores ordenados e um bit "sequencial a partir de zero"** para `ToString`/`Parse`
   rápidos (como `EnumInfo`).
5. **Não aloque para `hasFlag`**: faça a operação intrínseca/expandida no compilador.
6. **Defina semântica de valores desconhecidos** (permitir, rejeitar, avisar) e documente.
7. **Considere `Enum` constraint em genéricos** (`where T : enum`) e generic math sobre o tipo
   subjacente — habilitou código reutilizável no próprio CoreLib.
8. **Permita unbox/conversão entre enum e subjacente** apenas se quiser a mesma regra do .NET.

### Exemplo de comportamento

```csharp
[Flags] enum Perm : byte { None = 0, R = 1, W = 2, X = 4 }
(Perm.R | Perm.W).ToString();   // "R, W"      (HasFlagsAttribute -> FormatFlagNames)
((Perm)8).ToString();           // "8"         (bit sem nome -> número)
enum Cor { Vermelho, Verde }    // sem [Flags]
((Cor)1).ToString();            // "Verde"     (valores sequenciais a partir de zero -> índice direto)
((Cor)7).ToString();            // "7"
```

### Checklist de decisões

- [ ] Enum é inteiro nomeado ou tipo soma?
- [ ] Flags é um atributo (convenção) ou um tipo próprio verificado?
- [ ] Valores fora do conjunto são permitidos?
- [ ] Switch/match exige exaustividade?
- [ ] Como nomes e valores são acessíveis em runtime e a que custo?

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Enum.cs`
- `src/libraries/System.Private.CoreLib/src/System/Enum.EnumInfo.cs`
- `src/coreclr/System.Private.CoreLib/src/System/Enum.CoreCLR.cs`
- `src/coreclr/vm/methodtable.h` (`enum_flag_Category_Primitive`, `IsEnum`, `IsTruePrimitive`)
- `src/coreclr/vm/typedesc.cpp` (`g_pEnumClass` em `ConstrainedAsObjRefHelper`)
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs` (`Unbox_Helper`)
