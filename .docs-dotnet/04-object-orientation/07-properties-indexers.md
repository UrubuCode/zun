# Properties e indexers (açúcar sobre métodos)

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### Uma propriedade é um par (ou trio) de métodos mais metadado

Em IL, não existe "acesso a propriedade". Existe:

- um método `get_X` (retorna o valor) e/ou `set_X` (recebe `value`), marcados `specialname`;
- uma entrada nas tabelas `Property` e `PropertyMap` do metadado que *agrupa* esses métodos sob um nome e um tipo, mais `MethodSemantics` ligando cada método ao papel (getter/setter/outros) (C#/ECMA).

Para o runtime, getters e setters são métodos comuns: têm `MethodDesc`, slot, podem ser virtuais, abstratos, estáticos, selados, e entram no dispatch como qualquer outro. O doc `docs/design/coreclr/botr/method-descriptor.md` não distingue acessores. O loader também não tem tipo especial de MethodDesc para propriedade (confirmado indiretamente: as categorias de MethodDesc no BOTR são IL, Instantiated, FCall, PInvoke, EEImpl, Array, ComInterop, Dynamic; não existe "Property").

Prova no repo (CoreLib): `src/libraries/System.Private.CoreLib/src/System/Span.cs` (~linha 147) declara o indexer

```csharp
public ref T this[int index]
{
    [Intrinsic]
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    [NonVersionable]
    get { ... }
}
```

Os atributos `[Intrinsic]`, `[AggressiveInlining]`, `[NonVersionable]` são colocados no *acessor* (que é o método real), mostrando que a propriedade é só um nome para o getter.

### Indexers

Um indexer é uma propriedade com parâmetros. O compilador C# gera, por padrão, `get_Item(int)`/`set_Item(int, T)` e uma propriedade chamada `Item` (C#/ECMA). `IndexerNameAttribute` (`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IndexerNameAttribute.cs`) permite mudar o nome usado nos metadados (repo: classe selada `IndexerNameAttribute(string indexerName)`). O atributo `DefaultMemberAttribute` (C#/ECMA) marca qual é o indexer padrão para linguagens que o suportam.

### Retorno por referência

`Span<T>` retorna `ref T` no getter: o "valor" da propriedade é um endereço. Isso mostra que getter-como-método admite semântica que um campo não permite (retornar referência *controlada*, com checagem de limites antes), e que o IL não limita a propriedade a "ler/escrever valor".

### Auto-properties e campos de apoio

Uma auto-property `public int X { get; set; }` é gerada pelo compilador como par de acessores + campo oculto `<X>k__BackingField` (C#/ECMA). `init` é um setter com `modreq(IsExternalInit)` (C#/ECMA). O JIT faz inlining dos acessores triviais (inferência); a anotação `[NonVersionable]` no CoreLib garante que o inlining entre assemblies (inclusive ReadyToRun) seja seguro para esses acessores (repo: uso do atributo em `Object.cs`, `Span.cs`).

### Virtualidade, interface e herança

Como acessores são métodos, propriedades podem ser:

- `virtual`/`abstract`/`override`: cada acessor tem seu slot (arquivo 02). Pode-se sobrescrever só o getter se o setter não for sobrescrito (C#/ECMA).
- Membros de interface: `int X { get; }` em interface declara um método abstrato `get_X`; DIM permite corpo (arquivo 01).
- `static abstract` propriedades: ex. `static abstract TResult AdditiveIdentity { get; }` em `System/Numerics/IAdditiveIdentity.cs` (repo; arquivo 05).

### Chamada

Em C#, `obj.X` compila para `callvirt get_X` (ou `call` se o compilador sabe que não é virtual/nulo, C#/ECMA). Não há custo semântico adicional além de uma chamada de método, normalmente inlinada.

## Por quê

1. **Encapsulamento com sintaxe de campo.** Permite começar com um campo e depois adicionar lógica (validação, lazy, notificação) sem mudar a sintaxe de uso na *fonte*. Importante: a mudança continua sendo *quebra binária* (campo -> propriedade muda IL), então a vantagem é de fonte, não de ABI (C#/ECMA).
2. **Interfaces podem declarar "dados"** sem declarar estado: uma interface não pode ter campos de instância, mas pode requerer `Count { get; }`.
3. **Ferramentas e reflexão**: o metadado `Property` permite a data binding, serialização, designers e depuradores enumerar "propriedades" como conceito de primeira classe (C#/ECMA).
4. **Reuso do mecanismo de métodos**: o runtime não precisa de novo mecanismo de dispatch/visibilidade/override para propriedades; tudo é herdado dos métodos (consistente com a ausência de categoria "Property" no MethodDesc).

## Trade-offs

- **Ocultam custo**: `x.Length` pode ser O(n), pode lançar, pode alocar. Convenção de design: propriedades devem ser baratas e sem efeitos colaterais, mas nada impõe isso.
- **Semântica de valor vs referência**: propriedade em struct retorna cópia; mutar `s.Prop.Field = 1` é erro no C# ou muta cópia. Retorno `ref` mitiga.
- **Não são lvalues plenos**: não se pode passar propriedade por `ref` (a menos que retorne `ref`), nem usar em `out` direto; isso decorre de ser método.
- **Quebra binária ao trocar campo por propriedade**; ao contrário em linguagens com acesso uniforme verdadeiro (Eiffel, Scala, em certo grau).
- **Serialização/reflexão**: separar "campo" de "propriedade" duplica caminhos.
- **Indexers sem nome único** (`Item`) causam conflito com membros reais chamados `Item`, daí `IndexerName`.
- **Overhead em depuração**: step-into de getter trivial; a plataforma tem atributos (`DebuggerHidden`...) para mitigar (C#/ECMA).

## Como aplicar na sua linguagem

1. **Decida se propriedade é conceito de primeira classe do sistema de tipos ou açúcar de front-end.** O .NET faz as duas coisas: açúcar no IL (métodos) *mais* metadado (agrupamento) para ferramentas. A forma mais barata: açúcar puro (`x.y` -> `x.get_y()`), com um atributo ou tabela opcional para reflexão.
2. **Padronize o nome dos acessores** (`get_`/`set_`) e marque-os como "specialname" para evitar colisão com métodos escritos à mão.
3. **Permita visibilidade diferente por acessor** (`public get; private set;`): o .NET suporta porque a acessibilidade está no método.
4. **Defina o comportamento em interfaces** (declaração abstrata) e em herança (override de apenas um acessor).
5. **Considere retorno por referência** para indexers de coleções de valor (evita cópia e permite `v[i] = x` sem setter). O `Span<T>` do repo é o modelo.
6. **Inlining**: garanta que acessores triviais sejam inlinados mesmo entre módulos; o .NET precisou de `[NonVersionable]` para garantir isso em imagens pré-compiladas. Se sua linguagem tem compilação separada, projete um mecanismo equivalente ("este método nunca mudará").
7. **Decida a política de acesso uniforme**: se quer que trocar campo por propriedade seja compatível binariamente, faça *todo* campo público ser acessado via método (ou use linking em tempo de carga que resolve nomes para campo-ou-método).
8. **Indexers**: generalize como "operador `[]` sobrescrevível" (com sobrecarga por tipos de argumento). Evite exigir um nome único, mas reserve um nome interno.
9. **Auto-property**: gere o campo de apoio com nome não digitável, e permita inicializador.

Lowering de referência:

```
// fonte
prop Nome: string { get; set; }
// IR
field  _nome: string                    // nome não escrevível
method get_Nome(): string  { return this._nome }
method set_Nome(v: string) { this._nome = v }
metadata property Nome { getter: get_Nome, setter: set_Nome }
```

## Resumo dos fatos confirmados

- Propriedades são pares de métodos; não há categoria de MethodDesc "Property".
- Atributos como `[Intrinsic]`, `[MethodImpl(AggressiveInlining)]` e `[NonVersionable]` aparecem diretamente no getter do indexer de `Span<T>`.
- `Span<T>.this[int]` retorna `ref T` e checa limites com `(uint)index >= (uint)_length`.
- `IndexerNameAttribute` existe para renomear o par de acessores do indexer nos metadados.
- Propriedades `static abstract` são usadas nas interfaces de matemática genérica.

## O que o compilador gera (C#/ECMA)

1. Método `get_X` e/ou `set_X` com `specialname`.
2. Linha em `Property` + `PropertyMap` + `MethodSemantics`.
3. Se auto-property, campo `<X>k__BackingField` com `CompilerGenerated`.
4. `init`: setter com `modreq(IsExternalInit)`.
5. Indexer: nome padrão `Item`; `DefaultMemberAttribute` na classe.
6. Uso: `obj.X` -> `callvirt get_X`.

## Padrões e anti-padrões

- Bom: getter barato e idempotente, validação no setter, propriedade calculada pura.
- Ruim: getter que faz I/O, aloca coleção a cada leitura, ou lança em estado válido.
- Struct mutável com propriedade: cuidado com cópias (`s.P.X = 1`).
- Propriedade virtual lida em construtor: chama override antes de o filho inicializar (armadilha geral de OOP).
- Indexers múltiplos: sobrecarregue por tipo de parâmetro.
- Interface com propriedade: cada implementador decide se há campo de apoio.

## Perguntas de design

- Haverá acesso uniforme (campo e propriedade com a mesma sintaxe e compatibilidade binária)?
- Propriedades podem ser passadas como `ref`/`out`? (no .NET só se retornam `ref`)
- Visibilidade diferente por acessor?
- Propriedades com corpo padrão em interfaces?
- Indexador como sobrecarga de operador `[]` ou como membro nomeado?
- Metadados de propriedade são necessários em runtime (reflexão, serialização)?
- Como o linker/trimmer enxerga propriedades usadas só por reflexão?

## Glossário

- Acessor: método `get_X`/`set_X` que implementa a propriedade.
- Auto-property: propriedade cujo corpo é gerado, com campo de apoio oculto.
- Backing field: campo que armazena o valor de uma propriedade.
- Indexer: propriedade com parâmetros, em C# declarada com `this[...]`.
- `specialname`: bit de metadado que marca métodos gerados por construções de linguagem.
- Retorno por referência: getter que devolve `ref T` e permite escrita direta no elemento.
- `NonVersionable`: atributo que promete que o corpo do método nunca mudará, permitindo inlining entre versões.
- Intrinsic: marca que o JIT trata o método de forma especial.

## Notas finais

- Toda propriedade pode ser reescrita como métodos sem perda semântica.
- A única informação extra é o agrupamento no metadado.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Span.cs` (~147): indexer com `ref T`, `[Intrinsic]`, `[NonVersionable]`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IndexerNameAttribute.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Numerics/IAdditiveIdentity.cs`: propriedade `static abstract`.
- `docs/design/coreclr/botr/method-descriptor.md`: categorias de MethodDesc (sem "Property").
- `src/libraries/System.Private.CoreLib/src/System/Object.cs`: uso de `[NonVersionable]`.
