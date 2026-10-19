# Resolucao de bibliotecas nativas (NativeLibrary, DllImportResolver)

> Objetivo: documentar a cadeia de resolucao de `[DllImport]`/P/Invoke: o que roda antes da busca padrao, a busca em si (variacoes de nome, diretorios do host, diretorio do assembly, flags `DllImportSearchPath`), os ganchos por-assembly (`NativeLibrary.SetDllImportResolver`) e por-contexto (`LoadUnmanagedDll`, `ResolvingUnmanagedDll`), e como isso muda em single-file e NativeAOT.

## Mecanismo no .NET

### Onde o P/Invoke chega na resolucao

Na primeira chamada de um P/Invoke, `NDirect` resolve o alvo. Em `src/coreclr/vm/dllimport.cpp` (~linha 5939), a ordem e:

1. QCalls internos do runtime (`QCallResolveDllImport`).
2. **`PInvokeOverride::GetMethodImpl(libName, entryPoint)`** (`src/coreclr/vm/pinvokeoverride.cpp`): hosts podem registrar um override (propriedade/ponteiro `PINVOKE_OVERRIDE`, campo `pinvoke_override` em `src/native/corehost/host_runtime_contract.h`). Se devolver ponteiro, nao ha busca de biblioteca nenhuma. Exemplo de uso interno: a ponte Objective-C (`interoplibinterface_objc.cpp`, `Source::ObjectiveCInterop`). E o mecanismo de linkagem estatica de nativos em single-file/embarcados.
3. `NativeLibrary::LoadLibraryFromMethodDesc(pMD)` -> `LoadNativeLibrary` (descrito abaixo).
4. `PInvokeGetEntryPoint(pMD, hmod)` procura o simbolo (com decoracoes de nome na plataforma).

### `LoadNativeLibrary(PInvokeMethodDesc*)`: a ordem completa

`src/coreclr/vm/nativelibrary.cpp` (~linha 731):

```
1. LoadNativeLibraryViaDllImportResolver   -> callback registrado por NativeLibrary.SetDllImportResolver(assembly, ...)
2. LoadNativeLibraryViaAssemblyLoadContext -> AssemblyLoadContext.LoadUnmanagedDll (somente binder NAO-default)
3. pDomain->FindUnmanagedImageInCache(nome)  -> cache por nome ja resolvido
4. LoadNativeLibraryBySearch               -> busca padrao (abaixo); em sucesso, AddUnmanagedImageToCache
5. LoadNativeLibraryViaAssemblyLoadContextEvent -> evento AssemblyLoadContext.ResolvingUnmanagedDll (ultimo recurso)
```

Pontos verificaveis no codigo:

- Passo 2: se o binder do assembly `IsDefault()`, retorna `NULL` (o Default usa o mecanismo padrao). No Windows, nomes que comecam com `api-` ou `ext-` (API sets) **nao podem ser sobrescritos** (`IsWindowsAPISet`, "Prevent Overriding of Windows API sets").
- Passo 1: `LoadNativeLibraryViaDllImportResolver` invoca `NativeLibrary.LoadLibraryCallbackStub(libName, assembly, hasDllImportSearchPathFlags, flags)` em C#.
- Passo 5: so executa se existir um ALC gerenciado para o assembly (`GetAssemblyLoadContext(pAssembly) != 0`).

### API gerenciada: `NativeLibrary`

`src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/NativeLibrary.cs` (+ `NativeLibrary.Unix.cs`, `NativeLibrary.Windows.cs`; NativeAOT em `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/InteropServices/NativeLibrary.NativeAot.cs`):

| Membro | Semantica |
|---|---|
| `Load(string path)` / `TryLoad(path, out h)` | Carrega por caminho (como `dlopen`/`LoadLibrary`) |
| `Load(name, assembly, DllImportSearchPath?)` / `TryLoad(...)` | Executa a **mesma busca** que um P/Invoke do `assembly` faria (inclui ALC); exige `RuntimeAssembly` |
| `Free(h)` | `dlclose`/`FreeLibrary` |
| `GetExport(h, name)` / `TryGetExport` | `dlsym`/`GetProcAddress` |
| `SetDllImportResolver(assembly, DllImportResolver)` | **Um** resolvedor por assembly; segundo registro lanca `InvalidOperationException` |
| `GetMainProgramHandle()` | Handle do modulo do executavel principal |

`public delegate IntPtr DllImportResolver(string libraryName, Assembly assembly, DllImportSearchPath? searchPath);` -- `searchPath` e `null` quando o P/Invoke nao tem `[DefaultDllImportSearchPaths]`. O mapa fica em `ConditionalWeakTable<Assembly, DllImportResolver> s_nativeDllResolveMap` (nao impede coleta do assembly). Retornar `IntPtr.Zero` significa "nao resolvi, continue".

Uso tipico (multiplataforma, mapeando nomes):

```csharp
NativeLibrary.SetDllImportResolver(typeof(Interop).Assembly, (name, asm, searchPath) =>
{
    if (name != "mylib") return IntPtr.Zero;                         // deixa o runtime resolver o resto
    string rid = RuntimeInformation.RuntimeIdentifier;               // ex.: linux-x64
    string file = OperatingSystem.IsWindows() ? "mylib.dll" : OperatingSystem.IsMacOS() ? "libmylib.dylib" : "libmylib.so";
    string path = Path.Combine(AppContext.BaseDirectory, "runtimes", rid, "native", file);
    return NativeLibrary.TryLoad(path, out var h) ? h : IntPtr.Zero;
});
```

### Ganchos por contexto

Em `AssemblyLoadContext.cs`:

```csharp
protected virtual IntPtr LoadUnmanagedDll(string unmanagedDllName) => IntPtr.Zero;      // passo 2
protected IntPtr LoadUnmanagedDllFromPath(string unmanagedDllPath)                        // helper => NativeLibrary.Load(path)
public event Func<Assembly, string, IntPtr>? ResolvingUnmanagedDll;                      // passo 5
internal IntPtr GetResolvedUnmanagedDll(Assembly asm, string name)                       // primeiro handle != 0 vence
```

O VM chama `ResolveUnmanagedDll` e `ResolveUnmanagedDllUsingEvent` por `[UnmanagedCallersOnly]` (`AssemblyLoadContext.CoreCLR.cs`). `LoadUnmanagedDllFromPath` exige caminho totalmente qualificado (`IsPartiallyQualified` -> `ArgumentException`). O doc `assemblyloadcontext.md` resume: "se nao sobrescrito, ou se a resolucao nao resolve, o mecanismo padrao e usado como fallback".

Plugins usam `AssemblyDependencyResolver.ResolveUnmanagedDllToPath(name)` (`AssemblyDependencyResolver.cs`), que combina as **variacoes de nome** (`LibraryNameVariation.DetermineLibraryNameVariations`, copia gerenciada da logica de `DetermineLibNameVariations` em `dllimport.cpp`) com `_nativeSearchPaths` do `deps.json` do plugin; se o nome contem separador de diretorio, busca no diretorio do componente.

### A busca padrao: `LoadNativeLibraryBySearch`

Linhas ~541-660 de `nativelibrary.cpp`. Algoritmo:

```
para cada variacao de nome (prefixo "lib"/sufixo ".so"/".dylib"/".dll" conforme plataforma e se o nome ja tem sufixo/delimitador):
   1. LoadFromNativeDllSearchDirectories(variacao)       // NATIVE_DLL_SEARCH_DIRECTORIES do host = "well known path"
   2. se nome absoluto:  LoadLibrary(variacao, flags)
      senao se (assembly chamador && searchAssemblyDirectory):
            LoadFromPInvokeAssemblyDirectory(assembly, variacao)   // pasta do assembly do P/Invoke
   3. se (!userSpecifiedSearchFlags || !searchAssemblyDirectory || flags != 0):
            LoadLibrary(variacao, dllImportSearchPathFlags)        // padrao do SO (PATH / LD_LIBRARY_PATH / etc.)
```

Detalhes:

- **Variacoes de nome** (`LibraryNameVariation.Unix.cs`): `LibraryNamePrefix="lib"`, sufixo `.dylib` em Apple e `.so` nos demais. Se o nome ja contem o sufixo (`libfoo.so` ou `libfoo.so.1`), tenta nome exato primeiro; se nao, tenta primeiro `nome+sufixo`, depois `lib+nome+sufixo`, depois o nome puro. Prefixo `lib` so e testado se nao ha separador de diretorio. Nome com caminho absoluto gera uma unica variacao (nome exato). Os comentarios do codigo: "P/Invokes are often declared with variations on the actual library name... to smooth over cross-platform differences".
- **`DllImportSearchPath`** (`DllImportSearchPath.cs`) tem bit `AssemblyDirectory` (0x2) que no Windows colide com um flag de `LoadLibraryEx`; o codigo separa: `searchAssemblyDirectory` tratado pelo CLR e o restante repassado como `dllImportSearchPathFlags` (`DLLIMPORTSEARCHPATH_ASSEMBLYDIRECTORY`). Padrao sem atributo: busca no diretorio do assembly *e* padroes do SO (`searchAssemblyDirectory=true`, flags 0). `[DefaultDllImportSearchPaths]` por metodo, depois por assembly (`GetDllImportSearchPathFlags(pMD...)`, `(Module...)`).
- **Windows**: `LOAD_WITH_ALTERED_SEARCH_PATH` e removido quando incompativel com `LOAD_LIBRARY_SEARCH_*`; ha um atalho para API sets direto em `System32`.
- **Unix**: sem `LOAD_LIBRARY_SEARCH`; os diretorios vem de `NATIVE_DLL_SEARCH_DIRECTORIES` e do diretorio do assembly.
- **Cache**: `AddUnmanagedImageToCache(wszLibName, hmod)` evita repetir a busca.
- **Erro**: `LoadLibErrorTracker` acumula o motivo de cada tentativa e `errorTracker.Throw(...)` produz `DllNotFoundException` com todas as mensagens.

### De onde vem `NATIVE_DLL_SEARCH_DIRECTORIES`

`hostpolicy` -> `deps_resolver_t::resolve_probe_paths` -> `resolve_probe_dirs(native)` produz diretorios de assets `native` (e `runtimeTargets` com `assetType: native` do RID) para o app e frameworks, e entrega como propriedade (`coreclr.cpp`, ~linha 158). `hostpolicy.cpp` tem ate uma funcao "get-native-search-directories" (linha ~217) para hosts. Em single-file (.NET 10), `BUNDLE_EXTRACTION_PATH` indica onde nativos extraidos estao (`host-runtime-information.md`).

### P/Invokes estaticos x runtime pack

Muitos "P/Invokes" da propria BCL apontam para `libSystem.Native`, `libSystem.Security.Cryptography.Native.OpenSsl` etc.; sao nativos do framework, encontrados via `NATIVE_DLL_SEARCH_DIRECTORIES`/diretorio do assembly. Em single-file/AOT sao linkados estaticamente e resolvidos por override (item 2 do primeiro bloco).

### NativeAOT

`src/coreclr/nativeaot/System.Private.CoreLib/src/Internal/Runtime/CompilerHelpers/InteropHelpers.cs`, `ResolvePInvokeSlow` (~linha 257), implementa o equivalente com **a mesma ordem conceitual**:

1. `NativeLibrary.LoadLibraryCallbackStub(moduleName, callingAssembly, hasDllImportSearchPath, dllImportSearchPath)` (o `DllImportResolver` registrado).
2. `NativeLibrary.LoadBySearch(...)` com busca padrao (padrao `DllImportSearchPath.AssemblyDirectory` quando nao ha atributo).
3. `AssemblyLoadContext.GetLoadContext(callingAssembly).GetResolvedUnmanagedDll(...)` -- "AssemblyLoadContext as a last chance attempt" (como `ResolvingUnmanagedDll`); `LoadUnmanagedDll` virtual nao e consultado nessa versao simplificada.
4. Falha => `loadLibErrorTracker.Throw(moduleName)`.

P/Invokes com nome estatico podem ser linkados direto pelo compilador (`DirectPInvoke`, `Direct P/Invoke`) e nem passam por essa resolucao; os demais usam uma celula de fixup (`MethodFixupCell`) resolvida na primeira chamada. Ver arquivo 12.

### Teste e tracing

`src/tests/Loader/NativeLibs/` e `src/libraries/System.Runtime.InteropServices/tests` (busque `NativeLibrary`) exercitam as combinacoes. `DOTNET_` event tracing do loader (`AssemblyNative_Trace*`) cobre assemblies; para nativos a ferramenta usual e `strace -e openat`/Process Monitor, e o `DllNotFoundException` lista todas as tentativas.

## Por que

- **Nome logico no codigo, nome fisico por plataforma**: `[DllImport("sqlite3")]` deve funcionar em `sqlite3.dll`, `libsqlite3.so`, `libsqlite3.dylib`. As variacoes de nome (e nao um campo por SO) mantem uma unica declaracao.
- **Ordem de especificidade crescente**: override do host (static linking) -> resolvedor do assembly (decisao do autor da lib) -> ALC (decisao do host de plugins) -> busca padrao -> evento (ultimo recurso). Quem tem mais contexto decide primeiro.
- **Hooks distintos para donos distintos**: `SetDllImportResolver` e para quem escreve a **biblioteca** (um por assembly); `LoadUnmanagedDll`/`ResolvingUnmanagedDll` e para quem escreve o **host/plugin framework** (por contexto).
- **Default usa o caminho padrao**: `IsDefault() -> NULL` mantem o caminho feliz sem callback gerenciado, como na resolucao de assemblies.
- **API sets protegidos**: impedir que um ALC sobrescreva `api-ms-win-*` preserva o contrato do SO.
- **Cache por nome**: abrir bibliotecas e caro; o mapa nome->handle torna chamadas subsequentes baratas.

## Trade-offs

- **Muitos pontos de extensao** (5 estagios) -> dificil prever quem venceu; mitigado por mensagens de erro agregadas.
- **Um resolvedor por assembly** evita conflito entre bibliotecas, mas impede composicao (duas libs de terceiros querendo o mesmo assembly falham com `InvalidOperationException`).
- **Variacoes de nome geram falsos positivos**: `foo` pode carregar `libfoo.so` errado do `LD_LIBRARY_PATH`; seguranca de DLL planting no Windows (por isso `DllImportSearchPath`).
- **Busca depende do SO** (flags `LOAD_LIBRARY_SEARCH_*` so no Windows; `RPATH`/`LD_LIBRARY_PATH` em Linux), levando a comportamento nao identico entre plataformas.
- **Descarregamento**: bibliotecas nativas carregadas por um ALC coletavel nao sao descarregadas automaticamente com ele (o handle e do processo); quem chama `NativeLibrary.Load` deve chamar `Free`.
- **AOT**: sem `LoadUnmanagedDll` virtual e com linkagem estatica opcional, o comportamento difere do JIT.

## Como aplicar na sua linguagem

1. **Declare bibliotecas nativas por nome logico** e implemente variacoes de nome por plataforma (prefixo/sufixo) numa funcao pura testavel. Copie a ordem: nome exato se ja tem sufixo; senao com sufixo; senao com prefixo+sufixo; senao puro.
2. **Defina uma cadeia de resolucao fixa e documentada**, por exemplo: `override estatico do host` > `resolvedor por modulo` > `resolvedor por escopo` > `diretorios do manifesto (native)` > `diretorio do proprio modulo` > `padrao do SO` > `evento/ultimo recurso`. Retorne "nao resolvi" (nulo) para continuar.
3. **Permita linkagem estatica / override do host** (analogo a `PINVOKE_OVERRIDE`) -- e a ponte para binarios unicos e para sandboxes.
4. **Exija caminho absoluto nos helpers de carga** (como `LoadUnmanagedDllFromPath`) para evitar planting.
5. **Mantenha um cache nome->handle** e um agregador de erros ("tentei A, B, C") em vez de reportar so a ultima falha.
6. **Exponha `load/free/get_export/main_program_handle`** como biblioteca padrao (`NativeLibrary`) e use-a internamente, garantindo uma unica semantica.
7. **Decida quem libera**: ou o escopo descarregavel rastreia handles abertos e os libera no unload, ou documente que nao libera.
8. **Separe o diretorio de nativos por RID no manifesto** (`runtimes/<rid>/native`), para que o host passe diretorios corretos ao runtime sem logica em tempo de execucao.
9. **Proteja nomes de API do SO** contra sobrescrita.

## Referencias

- `src/coreclr/vm/nativelibrary.cpp`, `nativelibrary.h`, `nativelibrarynative.cpp`, `dllimport.cpp`, `pinvokeoverride.cpp`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/NativeLibrary.cs`, `NativeLibrary.Unix.cs`, `NativeLibrary.Windows.cs`, `DllImportSearchPath.cs`, `DefaultDllImportSearchPathsAttribute.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs`, `AssemblyDependencyResolver.cs`, `LibraryNameVariation*.cs`
- `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/InteropServices/NativeLibrary.NativeAot.cs`, `.../Internal/Runtime/CompilerHelpers/InteropHelpers.cs`
- `src/native/corehost/host_runtime_contract.h` (`pinvoke_override`, `NATIVE_DLL_SEARCH_DIRECTORIES`), `hostpolicy/deps_resolver.cpp`, `hostpolicy/coreclr.cpp`
- `docs/design/features/assemblyloadcontext.md` (PInvoke Resolution), `docs/design/features/dllmap.md`, `host-runtime-information.md`
- `src/tests/Loader/NativeLibs/`
- ECMA-335 Partition II 15.5 (Platform invoke / `pinvokeimpl`) e 23.1.8 (PInvokeAttributes, tabela `ImplMap`): define o P/Invoke, mas **nao** a estrategia de busca da biblioteca (implementation-defined).
