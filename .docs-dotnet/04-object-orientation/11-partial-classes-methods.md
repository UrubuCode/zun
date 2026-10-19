# Partial classes e partial methods

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### É um recurso de compilador, invisível ao runtime

`partial` não existe no IL nem no metadado. O compilador C# reúne todas as declarações `partial` do mesmo tipo (mesmo namespace, mesmo nome, mesma aridade genérica) *dentro da mesma compilação* e emite um único TypeDef (C#/ECMA). Para o CLR/`MethodTableBuilder`, o resultado é um tipo comum: nada em `src/coreclr/vm/methodtable.h` ou `methodtablebuilder.cpp` sabe de "partial". Consequência: partial não pode atravessar assemblies e não afeta layout, vtable ou dispatch.

Regras de mesclagem (C#/ECMA, não verificadas aqui): modificadores de acesso devem coincidir; `abstract`/`sealed` de qualquer parte aplicam-se ao tipo; interfaces e atributos são a união das partes; a classe base declarada em uma parte vale para todas; membros são a união.

### Uso maciço no próprio repositório (repo)

O CoreLib usa partial como mecanismo central de *composição por plataforma e por runtime*:

1. **`System.Object`** é `public partial class Object` em dois arquivos:
   - `src/libraries/System.Private.CoreLib/src/System/Object.cs` (comum a todos os runtimes: `ToString`, `Equals`, `GetHashCode`, `ReferenceEquals`, finalizador);
   - `src/coreclr/System.Private.CoreLib/src/System/Object.CoreCLR.cs` (apenas CoreCLR: `GetType`, `MemberwiseClone`, que dependem de `MethodTable` e do alocador do CoreCLR).
   O csproj do CoreCLR inclui explicitamente `Object.CoreCLR.cs` (`src/coreclr/System.Private.CoreLib/System.Private.CoreLib.csproj` ~149). Outros runtimes (NativeAOT, Mono) fornecem as suas partes, mantendo o resto comum.
2. **`String`** está espalhada: `String.cs`, `String.Comparison.cs`, `String.Manipulation.cs`, `String.Searching.cs` (em `src/libraries/System.Private.CoreLib/src/System/`), listados em `System.Private.CoreLib.Shared.projitems` (~1227-1229). A divisão é por *tema*, não por plataforma.
3. **`Thread`** é `public sealed partial class Thread` em arquivos como `Thread.CoreCLR.cs`, `Thread.CoreCLR.Multithreading.cs`, `Thread.CoreCLR.SingleThreading.cs` (CoreCLR) e `Thread.NativeAot.cs`, `Thread.NativeAot.Unix.cs`, `Thread.NativeAot.Windows.cs` (NativeAOT): partial por runtime *e* por SO *e* por modo (multithread/WASM).
4. **`SafeFileHandle`**: `SafeFileHandle.cs` (comum), `SafeFileHandle.Unix.cs` e `SafeFileHandle.Windows.cs`; o projitems inclui o arquivo certo para cada SO (`System.Private.CoreLib.Shared.projitems` ~93 e ~2353). 
5. **`Enumerable`** (LINQ): `public static partial class Enumerable` repartida em `Aggregate.cs`, `AggregateBy.cs`, `Where.cs`, etc. em `src/libraries/System.Linq/src/System/Linq/` (arquivo 12).

### Partial methods

Existem duas formas:

- **Clássica (C# 3)**: `partial void M();` numa parte e opcionalmente o corpo em outra. Restrições: retorno `void`, sem modificadores de acesso, sem `out`; se não há implementação, o compilador remove a declaração *e as chamadas* (C#/ECMA). (Não verifiquei no repo um exemplo dessa forma clássica.)
- **Estendida (C# 9)**: com modificador de acesso e retornos não-void; *obrigatória* a implementação (C#/ECMA). É o que o repo mais usa: em `SafeFileHandle.cs` (~31) a declaração é `public static partial void CreateAnonymousPipe(out SafeFileHandle readHandle, out SafeFileHandle writeHandle, bool asyncRead = false, bool asyncWrite = false);` e a implementação aparece em `SafeFileHandle.Unix.cs` (~183) e `SafeFileHandle.Windows.cs` (~25) (repo). A API pública é declarada e documentada uma vez, e a implementação varia por plataforma; trocar a plataforma = trocar qual arquivo é compilado.

### Partial methods como ponto de extensão para geradores de código

A segunda grande função: um *source generator* gera uma das partes. Exemplos no repo:

- `[LibraryImport]` (P/Invoke gerado): em `RuntimeHelpers.CoreCLR.cs`, declarações como `[LibraryImport(QCall, EntryPoint = "ObjectNative_GetHashCodeSlow")] private static partial int GetHashCodeSlow(ObjectHandleOnStack o);` têm o corpo gerado (marshalling) pelo gerador (repo, ~246-250).
- `[GeneratedRegex]` (`src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/GeneratedRegexAttribute.cs`): o atributo se aplica a método ou propriedade `partial`, e o gerador cria o corpo (o `AttributeUsage` do repo é `Method | Property`) (repo). O padrão é: escrever "o quê" (declaração parcial + atributo), o gerador produz "o como" em outra parte.

### Atributos e documentação em partes

Para métodos parciais, atributos de ambas as partes são combinados; XML docs podem vir de qualquer uma (C#/ECMA, não verificado aqui).

## Por quê

1. **Separar código escrito à mão de código gerado**, para que designers/ferramentas (WinForms, WPF, EF, source generators) possam regenerar um arquivo sem tocar no do usuário. Este foi o motivo original (C#/ECMA).
2. **Organização de tipos enormes** (como `String`, `Enumerable`, `Thread`) em arquivos coesos, sem mudar o modelo de tipos.
3. **Variação por plataforma/runtime sem `#if` espalhados.** O repo usa o conjunto de arquivos incluídos no projeto como seletor de implementação (`projitems`/`csproj`). Ex.: um tipo `Object` comum + partes do runtime.
4. **API estável com implementações trocáveis**: a assinatura do método estendido é única, e a implementação é exigida em tempo de compilação (um erro se faltar), em contraste com interfaces + injeção.
5. **Custo zero em runtime**: tudo é resolvido na compilação; não há indireção.

## Trade-offs

- **Navegabilidade**: o leitor precisa abrir vários arquivos para ver um tipo completo; ferramentas ajudam (Go to Definition mostra lista).
- **Erros só na compilação do conjunto completo**: faltar uma parte = erro de "partial method sem implementação" (no formato estendido) ou, pior, remoção silenciosa (no formato clássico).
- **Ordem de inicialização e de membros indefinida entre partes** (C#/ECMA: a ordem de campos entre arquivos não é especificada, o que afeta layout sequencial e inicializadores de campo).
- **Escapa à coesão**: partial pode acobertar classes grandes demais (code smell).
- **Não atravessa assemblies**: não serve para estender tipos de outra biblioteca (para isso há extension methods: arquivo 12).
- **Mistura com `#if`** e arquivos específicos por plataforma complica builds e análise estática.
- **Versionamento**: como é compilação, não existe "adicionar uma parte" a um tipo já compilado.

## Como aplicar na sua linguagem

1. **Decida se quer partial.** Se sua linguagem tem geração de código de primeira classe (macros, source generators, codegen de UI), partial é muito útil. Sem geração de código, o benefício é principalmente organizacional.
2. **Mantenha-o como recurso de compilação**: o formato de binário/metadado não precisa conhecer partial. Isso é o que o .NET faz, e evita impacto no loader, vtable e dispatch.
3. **Escopo da junção**: um módulo/compilação. Defina como o compilador reúne partes (mesmo nome qualificado) e detecte conflitos de modificadores, classe base e membros duplicados.
4. **Defina a ordem de membros e a inicialização**: ou prometa ordem determinística (por ordem de arquivos na compilação), ou declare indefinida e proíba depender dela (ex.: layout explícito exige uma só parte).
5. **Offereça "declaração + implementação" para métodos** (partial method estendido) com *implementação obrigatória* e erro claro se faltar. Evite a variante antiga "some silenciosamente" a não ser que queira o padrão "hook opcional" (nesse caso, exija `void`, sem retorno).
6. **Integre com geradores**: permita que uma ferramenta emita uma parte a partir de atributos. É o padrão `[LibraryImport]`/`[GeneratedRegex]` do repo.
7. **Considere alternativas**: (a) *extension/impl blocks* (Rust `impl`, Swift `extension`) que permitem adicionar métodos em outro arquivo/módulo sem a palavra `partial`; (b) *classes abertas* (Ruby). Elas atravessam módulos, partial não.
8. **Para variação por plataforma**, avalie um mecanismo de *seleção de arquivos* do build, como o .NET, em vez de `#if`. Expõe a separação de forma estrutural.
9. **Ferramentas**: garanta que depuradores, IDE e docs agreguem as partes e que o gerador de documentação dedupe.

Sintaxe sugerida para sua linguagem:

```
// API.xyz
type Arquivo partial {
    fn abrir(caminho: str) -> Handle partial;     // só declaração
}
// API.windows.xyz
type Arquivo partial {
    fn abrir(caminho: str) -> Handle { ... }       // implementação
}
```

## Resumo dos fatos confirmados

- `Object`, `String`, `Thread`, `SafeFileHandle` e `Enumerable` são tipos `partial` no repo.
- A seleção de partes por plataforma/runtime é feita pela lista de arquivos nos projetos (`csproj`, `projitems`).
- `partial` com modificador de acesso (formato estendido) é usado para API única + implementação por plataforma.
- `[LibraryImport]` e `[GeneratedRegex]` usam métodos `partial` para o gerador de código preencher o corpo.
- O VM do CoreCLR não conhece `partial`.

## Pipeline típico

1. O autor escreve a declaração `partial` com atributo (ex.: `[LibraryImport]`).
2. O compilador roda o source generator.
3. O gerador emite outra parte do mesmo tipo com a implementação.
4. O compilador mescla as partes num TypeDef único.
5. O IL resultante não indica a divisão.

## Padrão "um arquivo por plataforma"

- Arquivo comum: declara o tipo, os campos e os métodos `partial` (sem corpo).
- Arquivo `.Unix.cs`: implementa os métodos para Unix.
- Arquivo `.Windows.cs`: implementa para Windows.
- O projeto inclui só um dos arquivos específicos, conforme o alvo.
- Vantagem: a API pública é única e documentada em um lugar.
- Risco: esquecer uma plataforma só falha ao compilar para ela.

## Perguntas de design

- A mesclagem de partes é por arquivo, módulo ou pacote?
- Qual a ordem de inicialização de campos entre partes?
- Atributos e comentários de documentação são mesclados?
- O compilador diferencia "parte gerada" de "parte escrita"?
- Existe verificação de que todas as declarações têm implementação?
- O mecanismo funciona com layout explícito de campos?
- As ferramentas (debugger, IDE) mostram o tipo inteiro?

## Glossário

- Partial type: tipo declarado em várias partes que o compilador funde.
- Partial method: método com declaração e implementação separadas.
- Source generator: componente do compilador que emite código-fonte adicional.
- `LibraryImport`: atributo que gera código de marshalling para chamadas nativas.
- `GeneratedRegex`: atributo que gera código de expressão regular em tempo de compilação.
- Projitems: arquivo MSBuild que lista fontes compartilhados entre projetos.
- Arquivo por plataforma: convenção de sufixo (`.Unix.cs`, `.Windows.cs`) para implementações específicas.
- QCall: convenção do CoreCLR para chamar código nativo a partir do CoreLib (usada com `LibraryImport`).

## Protótipo mínimo (pseudocódigo, proposta minha)

```
// arquivo: fila.lang
type Fila partial {
    campo itens: [int]
    fn adicionar(x: int) { itens.push(x) }
    fn salvar(caminho: str) partial        // declaração
}

// arquivo: fila.unix.lang
type Fila partial {
    fn salvar(caminho: str) { posix_write(caminho, itens) }
}

// arquivo: fila.windows.lang
type Fila partial {
    fn salvar(caminho: str) { win32_write(caminho, itens) }
}
```

Regras sugeridas:

- Exatamente uma implementação por declaração `partial` por compilação.
- Erro se faltar ou se houver duas.
- Modificadores e assinatura idênticos entre declaração e implementação.
- Campos só declarados em uma parte, ou ordem definida pela ordem dos arquivos.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Object.cs` e `src/coreclr/System.Private.CoreLib/src/System/Object.CoreCLR.cs`; `src/coreclr/System.Private.CoreLib/System.Private.CoreLib.csproj` (~149).
- `src/libraries/System.Private.CoreLib/src/System/String*.cs` e `System.Private.CoreLib.Shared.projitems` (~1227-1229).
- `src/coreclr/System.Private.CoreLib/src/System/Threading/Thread.CoreCLR*.cs`; `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Threading/Thread.NativeAot*.cs`.
- `src/libraries/System.Private.CoreLib/src/Microsoft/Win32/SafeHandles/SafeFileHandle.cs`, `.Unix.cs`, `.Windows.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs` (`[LibraryImport]` + `partial`).
- `src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/GeneratedRegexAttribute.cs`.
- `src/libraries/System.Linq/src/System/Linq/*.cs` (`Enumerable` partial).
