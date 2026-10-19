# Layout de tipo, StructLayout e alinhamento

## Mecanismo no .NET

### 1. Quem calcula o layout

O layout de campos é computado pelo **carregador de tipos** ao construir o `MethodTable`:
`MethodTableBuilder` em `src/coreclr/vm/methodtablebuilder.cpp`, com a parte de layout
"marshalável" em `src/coreclr/vm/classlayoutinfo.cpp` e `class.h` (`EEClassLayoutInfo`). No
tooling AOT há uma reimplementação em C# (`docs/design/coreclr/botr/managed-type-system.md`: o type
system gerenciado calcula "static and instance field layout" e "GC layout").

### 2. Três modos: `LayoutKind`

`src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/LayoutKind.cs`:

```csharp
public enum LayoutKind { Sequential = 0, Extended = 1, Explicit = 2, Auto = 3 }
```

`StructLayoutAttribute` tem `Pack`, `Size` e `CharSet` (`StructLayoutAttribute.cs`).
No `MethodTableBuilder` (~8260-8320):

- **Auto**: o runtime reordena campos livremente (`HandleAutoLayout`, ~8364). Padrão para
  classes (C#).
- **Sequential**: respeita a ordem de declaração (`HandleSequentialLayout`). Padrão de structs em C#.
- **Explicit**: cada campo tem offset explícito (`FieldOffset`), sobreposição permitida
  (`HandleExplicitLayout`, `HasExplicitFieldOffsetLayout`).
- **Extended**: valor presente no enum; não aprofundado aqui.

### 3. Regra importante: Sequential pode virar Auto

Código real (`methodtablebuilder.cpp` ~8265-8273):

```cpp
if (bmtLayout.layoutType == EEClassLayoutInfo::LayoutType::Sequential)
{
    // If the parent type is not Object, ValueType, or Sequential, or if this type has GC fields,
    // we will use Auto layout instead of Sequential layout and set the packing size.
    if ((hasNonTrivialParent && !pParentMT->IsManagedSequential()) || hasGCFields)
    {
        bmtLayout.layoutType = EEClassLayoutInfo::LayoutType::Auto;
        pLayoutInfo->SetPackingSize(bmtLayout.packingSize);
    }
}
```

Ou seja: um tipo marcado `Sequential` que **contém referências gerenciadas** é tratado como
**Auto** (o GC e o layout gerenciado se beneficiam de reordenar, por exemplo agrupando refs). Isso
significa que a ordem em memória *não é garantida* nesses casos — apenas para tipos blittable/sem
GC. O comentário também afirma que "auto-layout ignores any Pack directives" nesse caso. Por isso
`IsManagedSequential()` e `IsBlittable()` (`class.h` ~383, 1395, 1954-1975) são conceitos
distintos que o runtime consulta antes de permitir interop por memcpy.

### 4. Alinhamento

`classlayoutinfo.cpp` (`InitializeLayoutFieldInfoArray`, ~linhas 100-170):
- Campos que contêm ponteiros GC têm alinhamento de `TARGET_POINTER_SIZE`.
- Campos de struct aninhado usam `GetFieldAlignmentRequirement()` do tipo aninhado.
- Com `FEATURE_64BIT_ALIGNMENT`, tipos que `RequiresAlign8()` forçam alinhamento 8.
- O alinhamento de cada campo é `min(requisito, packingSize)`; o alinhamento do tipo é o
  máximo dos campos.
- `methodtablebuilder.cpp` ~8290: "For types with layout we drop any 64-bit alignment requirement if
  the packing size was less than 8 bytes (this mimics what the native compiler does and ensures we
  match up calling conventions during interop)".
- Flags de aninhamento (`class.h` ~540): `NonBlittable`, `GCPointer`, `Align8`, `AutoLayout`,
  `Int128`, `DecimalFloatingPoint`.

### 5. Layout explícito e GC

Em `Explicit`, campos de referência não podem sobrepor campos não-ref de modo inseguro:
`CheckValueClassLayout` e `ExplicitFieldTrust` (`methodtablebuilder.cpp` ~9122) determinam o nível
de "confiança"; se o layout explícito sobrepõe uma referência a dados arbitrários o tipo é
rejeitado (`kNone`). `HandleGCForExplicitLayout` calcula o mapa de GC (~1833).

### 6. Layout versus tamanho de objeto

Para classes, o objeto tem header + ponteiro de MethodTable (`type-loader.md`: ponteiro de MT "no
offset 0" e campos depois). Structs não têm header; quando boxed, os campos começam após o
ponteiro de MT (`GetRawData`, ver `06`). Para structs com `StructLayout(Size=N)`, `SetHasExplicitSize`
(~8263) registra o tamanho.

### 7. `InlineArray`

`InlineArrayAttribute(int length)` replica o layout do struct `length` vezes, inclusive o
**mapa de GC**, com ordem e empacotamento idênticos aos de elementos de um array
(`docs/design/features/InlineArrayAttribute.md`; `methodtablebuilder.cpp` ~1716-1723
`NumInlineArrayElements`, `SetIsInlineArray`). Não pode ter layout explícito; erros de construção
resultam em `TypeLoadException`.

### 8. Padding e igualdade

O runtime usa a análise de layout para saber se `Equals`/`GetHashCode` pode comparar bits
(`ValueType.CanCompareBitsOrUseFastGetHashCode`, `ValueType.cs` ~73-115): structs com padding ou
campos de ponto flutuante/ref não usam a comparação rápida.

## Por quê

- **Interop**: P/Invoke e COM exigem layout idêntico ao C. `Sequential`+`Pack` espelha `#pragma pack`.
- **Densidade e desempenho**: `Auto` permite ao runtime agrupar referências/reordenar para reduzir
  padding e simplificar o GC.
- **Controle total**: `Explicit` permite uniões C e acesso a formatos binários.
- **Segurança do GC**: layouts com referências sobrepostas a dados são rejeitados.
- **Compatibilidade de ABI**: a regra "descarta alinhamento 8 se Pack < 8" evita divergência de
  convenção de chamada com o compilador nativo.

## Trade-offs

- **Dois conceitos que parecem iguais** (Sequential vs managed-sequential): programadores são
  surpreendidos quando `Sequential` + campo de referência não é de fato sequencial.
- Layout `Auto` não é estável entre versões/arquiteturas — não serve para serialização binária.
- `Explicit` dá poder, mas facilita comportamento indefinido e é restrito para refs.
- Alinhamento 64-bit especial em plataformas 32-bit (`FEATURE_64BIT_ALIGNMENT`) adiciona ramos no
  alocador e no JIT.
- Cálculo de layout em tempo de carga custa tempo de startup (R2R precisa validar assumptions de
  layout — por isso o conceito `NonVersionable` em tipos como `Nullable<T>`).

## Como aplicar na sua linguagem

1. **Separe "layout interno" de "layout externo"**: default = livre para o compilador reordenar
   (como Auto); opt-in `repr(C)`/`Sequential` para interop.
2. **Defina cedo quais tipos são "blittable"** (sem refs gerenciadas, sem tipos não-trivials) — só
   eles garantem ordem e memcpy.
3. **Registre no descritor de tipo**: tamanho, alinhamento, mapa de GC (offsets de refs),
   blittable?, tem padding?
4. **Aplique `min(align, pack)`** se oferecer `pack`, mas cuide para que `pack` não viole o
   alinhamento de ponteiros GC (o runtime faz GC pointers = alinhados a ponteiro).
5. **Se oferecer uniões/layout explícito**, proíba sobreposição de referências com dados.
6. **Ofereça arrays inline** (`InlineArray`-like) com layout igual ao de arrays para permitir views
   seguras.
7. **Teste ABIs**: struct passado por valor em calling conventions diferentes depende de layout e
   alinhamento — gere testes com o compilador nativo da plataforma.

### Exemplo: padding e reordenação

```csharp
struct P { byte a; int b; byte c; }   // Sequential: a@0, b@4, c@8, tamanho 12 (alinhamento 4)
// Pack=1 -> tamanho 6, sem padding (min(alinhamento do campo, packingSize) = 1)
class Q { byte a; string s; int b; }  // Auto: o runtime pode agrupar a referência e reordenar
struct R { byte a; string s; }        // Sequential + campo GC => tratado como Auto pelo carregador
```

(Os números do primeiro caso seguem a regra `min(alinhamento, pack)` lida em
`classlayoutinfo.cpp`; o tamanho 12 é a aplicação aritmética típica, não medida aqui.)

### Checklist de decisões

- [ ] Layout default: livre ou declarado?
- [ ] Como indicar layout C-compatível e qual a política quando há referências GC?
- [ ] Quais metadados de layout o descritor de tipo guarda (tamanho, align, GC map, blittable)?
- [ ] Layout explícito: quais sobreposições são seguras?
- [ ] Como arrays inline preservam o layout de array?

## Referências

- `src/coreclr/vm/methodtablebuilder.cpp` (~1716-1723, 1833, 4210-4350, 8260-8370, 9122)
- `src/coreclr/vm/classlayoutinfo.cpp`, `src/coreclr/vm/class.h` (`EEClassLayoutInfo`, `IsBlittable`, `IsManagedSequential`)
- `src/coreclr/vm/methodtable.h` (`ContainsGCPointers`, `RequiresAlign8`)
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/LayoutKind.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/StructLayoutAttribute.cs`
- `docs/design/features/InlineArrayAttribute.md`
- `docs/design/coreclr/botr/managed-type-system.md`
- `src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`
