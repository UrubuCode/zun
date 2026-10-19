# Modificadores de acesso e encapsulamento

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento geral não verificado aqui; "(inferência)" = dedução minha.

## Mecanismo no .NET

### Os níveis existem no metadado, não só na linguagem

O CLI define sete níveis de acessibilidade para membros (e variantes para tipos). Os nomes de metadados que o runtime usa aparecem em `src/coreclr/vm/clsload.cpp`: `mdPrivate`, `mdFamily`, `mdAssem`, `mdFamORAssem`, `mdFamANDAssem`, `mdPublic` (membros) e, para tipos aninhados, `tdNestedPublic`, `tdNestedFamily`, `tdNestedPrivate`, `tdNestedFamORAssem`, `tdNestedFamANDAssem`, `tdNestedAssembly` (repo, `ClassLoader::CanAccessClass`, ~4133-4263).

Mapeamento para C# (C#/ECMA, não verificado aqui):

| C# | CLI |
|---|---|
| `public` | public |
| `private` | private |
| `protected` | family |
| `internal` | assembly |
| `protected internal` | family OR assembly |
| `private protected` | family AND assembly |
| (nível de arquivo `file`) | tratado pelo compilador (tipo internal com nome mangled) |

### Onde o runtime verifica

O runtime *aplica* acessibilidade em momentos específicos, não a cada acesso:

- **Em JIT/resolução de tokens**: quando o JIT (ou o loader) resolve uma chamada, campo ou tipo, `ClassLoader::CanAccess`, `CheckAccessMember`, `CanAccessClass` e `CanAccessFamily` (`src/coreclr/vm/clsload.cpp`, ~4075-4600) decidem se o chamador pode ver o alvo (repo). O código gerado depois não repete a checagem; a verificação é feita na primeira resolução (inferência; é o padrão de lazy binding do runtime).
- **Contexto de acesso**: `class AccessCheckContext` (`clsload.hpp` ~282) carrega (método chamador, tipo chamador, assembly chamador). `AccessCheckOptions` (~332) escolhe se a falha lança exceção ou só devolve falso, e se há demanda de permissão para reflexão (repo).
- **Family (protected)**: `CanAccessFamily(pCurrent, pTarget)` sobe pela cadeia de pais de `pCurrent` (`GetParentMethodTable()`) e pelos tipos que o contêm (`GetEnclosingMethodTable`) procurando o tipo alvo. Para interfaces, varre o interface map. Comentário do código: o compilador C# é "incredibly lax about generics", então basta ser subclasse de *qualquer* instanciação do genérico, e o runtime espelha isso (repo).
- **Assembly vs. friend**: `AssemblyOrFriendAccessAllowed(...)` considera `InternalsVisibleTo` (e `IgnoresAccessChecksTo`) (repo: `clsload.cpp`, `assembly.cpp` ~2477, `comdynamic.cpp` ~857).
- **Tipos aninhados**: acessibilidade do tipo aninhado é traduzida em acessibilidade de membro do tipo externo (repo, switch em `CanAccessClass`): `tdNestedFamORAssem` degrada para `public` se há acesso por assembly/friend, senão exige family.
- **Overrides**: `TestOverRide` no `MethodTableBuilder` valida que o override não reduz acessibilidade de forma incompatível (repo; chamado em `methodtablebuilder.cpp` ~5420; regra exata não lida em detalhe).

### O que *não* é encapsulamento no .NET

- **Reflexão** pode acessar membros privados (sujeito a permissão em plataformas antigas; hoje em geral livre em full-trust). Existe `UnsafeAccessorAttribute` como caminho oficial e verificado para acesso a membros privados sem reflexão (`docs/design/features/unsafeaccessors.md`, repo).
- **Layout de objeto** é observável: `Unsafe`, ponteiros e `Marshal` leem campos privados de structs.
- **Encapsulamento é por tipo/assembly, não por instância**: um método de `Foo` pode acessar o campo privado de *outra* instância de `Foo` (C#/ECMA).
- Para generics e membros em interfaces, acessibilidade também vale: membros não públicos em interfaces com DIM (C#/ECMA).

### Acessibilidade e a vtable

Acessibilidade não muda o layout da vtable: um método `private virtual` (possível em IL, e `protected` virtual comuns) ainda ocupa slot. `MethodImpl` não afeta visibilidade do decl: o comentário em `methodtablebuilder.cpp` (~4910) diz "Since MethodImpl's do not affect the visibility of the Decl method..." (repo), e implementação explícita de interface torna o corpo privado mas o método da interface visível via interface.

## Por quê

1. **Integridade e manutenção**: restringir superfície pública permite mudar implementação sem quebrar clientes.
2. **Modelo de assembly como unidade de implantação**: `internal`/`assembly` e `InternalsVisibleTo` refletem que o assembly é a unidade de versionamento e distribuição. O runtime embute esse conceito (repo: `AssemblyOrFriendAccessAllowed`).
3. **Herança exige um nível intermediário** (`protected`): `CanAccessFamily` caminha pela cadeia de pais, ligando o conceito à herança única.
4. **Verificação lazy e uma vez só**: custo zero no caminho quente.
5. **Segurança (histórica)**: acessibilidade foi parte do modelo de verificação de código/CAS (inferência; o repo contém referências remanescentes mas não detalhei).

## Trade-offs

- **Sete níveis** (dois deles raros: `family AND assembly`, `family OR assembly`) aumentam a complexidade do loader (switch extenso em `CanAccessClass`).
- **Verificar na resolução** significa que código IL malicioso/manual pode ser rejeitado só em tempo de JIT, com exceções como `MethodAccessException`/`FieldAccessException` em runtime e não em carga (C#/ECMA).
- **Reflexão e `UnsafeAccessor`** fazem o encapsulamento ser uma barreira de *design*, não de *segurança*.
- **`InternalsVisibleTo`** abre brechas controladas; facilita testes, mas aumenta acoplamento.
- **Genéricos**: regra "frouxa" de family para genéricos (comentário em `CanAccessFamily`) é uma concessão por compatibilidade com o compilador.
- **Acesso por tipo, não por instância** simplifica `Equals`/clone, mas elimina o conceito de "estado realmente privado ao objeto".

## Como aplicar na sua linguagem

1. **Escolha a unidade de encapsulamento**: tipo, módulo/arquivo, pacote/assembly. O .NET usa tipo e assembly. Se sua linguagem tem módulos como unidade de compilação, `internal` = módulo.
2. **Mínimo viável**: `public`, `private` (tipo), `module` (assembly) e, se há herança, `protected`. Os níveis combinados (`OR`/`AND`) raramente valem o custo.
3. **Registre o nível no metadado do binário**, não só no compilador. O .NET permite que outras linguagens e ferramentas (reflexão, outros compiladores) respeitem as mesmas regras.
4. **Decida onde verificar**: no compilador (rápido, mas não protege contra binários forjados) e/ou no carregador/JIT (como o `ClassLoader::CanAccess`). Se admite carregar código não confiável, verifique na carga; senão, só no compilador.
5. **Defina um escape hatch oficial** (como `UnsafeAccessor` ou `friend`) em vez de deixar apenas reflexão. Permite testes e serialização sem quebrar o modelo.
6. **Regra de override vs. acessibilidade**: um override pode alargar a visibilidade? Reduzir? O `TestOverRide` do .NET é o lugar para essa política; defina-a explicitamente.
7. **Campos**: considere proibir campos públicos mutáveis por padrão (ou torná-los açúcar para propriedades; ver arquivo 07), porque acessibilidade de campo não permite lógica no acesso.
8. **Defina o tratamento de tipos aninhados**: o aninhado vê privados do externo? O externo vê privados do aninhado? (No .NET, o aninhado tem acesso aos membros do externo; o inverso não vale: C#/ECMA.)
9. **Visibilidade em interfaces** com implementação padrão: deixe claro se membros não públicos são permitidos.

Exemplo de tabela de decisão para sua linguagem:

```
nível      mesmo tipo   subtipo   mesmo módulo   outro módulo
private    sim          não       não            não
protected  sim          sim       não            não
module     sim          sim/n     sim            não
public     sim          sim       sim            sim
```

## Resumo dos fatos confirmados

- Níveis em metadado: `mdPrivate`, `mdFamily`, `mdAssem`, `mdFamORAssem`, `mdFamANDAssem`, `mdPublic`; tipos aninhados com `tdNested*`.
- O loader verifica acesso com `ClassLoader::CanAccess*` e `CheckAccessMember`.
- Contexto: `AccessCheckContext` (método, tipo e assembly chamadores).
- `AccessCheckOptions` decide se falha lança ou apenas retorna falso.
- `CanAccessFamily` percorre pais e tipos envolventes; interfaces via interface map.
- A regra de família é frouxa com genéricos, para imitar o compilador C#.
- Amigos: `InternalsVisibleTo` e `IgnoresAccessChecksTo`.
- `UnsafeAccessor` é o caminho oficial para acesso a privados sem reflexão.

## Onde cada tipo de violação aparece (modelo)

- Fonte C# usa membro privado de outro tipo: erro do compilador.
- IL manual usa membro privado: exceção de acesso na resolução do token (JIT/loader).
- Override reduz visibilidade: rejeitado na carga do tipo (`TestOverRide`).
- Reflexão acessa privado: em geral permitido.

(Itens 1 e 4: C#/ECMA; itens 2 e 3 decorrem do código citado, sem teste de execução.)

## Perguntas de design

- A unidade de encapsulamento é tipo, módulo ou pacote?
- `protected` dá acesso apenas por instância do próprio tipo derivado (regra do C#) ou a qualquer instância da base?
- Há "friend" explícito? Como é declarado e auditado?
- Acesso a privados via reflexão/FFI é permitido? Em que modo de compilação?
- Visibilidade influencia otimização? Provavelmente: privado e selado permitem inlining mais agressivo (inferência).
- Interfaces podem declarar membros não públicos?
- Visibilidade de tipo aninhado vs. visibilidade do tipo externo: definir a interseção.
- Há um jeito de testar código interno sem abrir a API pública? (`InternalsVisibleTo` é a resposta do .NET)

## Glossário

- Family: acessível ao próprio tipo e a tipos derivados (C# `protected`).
- Assembly: acessível dentro do mesmo assembly (C# `internal`).
- FamORAssem: family ou assembly (C# `protected internal`).
- FamANDAssem: family e assembly (C# `private protected`).
- Friend assembly: assembly autorizado a ver membros internos (`InternalsVisibleTo`).
- Caller context: o trio método/tipo/assembly de quem solicita o acesso.
- Visibility check: parte da verificação que valida acesso a tipos.
- Member access check: parte da verificação que valida acesso a membros.
- `UnsafeAccessor`: mecanismo oficial para acessar membros inacessíveis de forma verificada.

## Protótipo mínimo de verificação de acesso (pseudocódigo, proposta minha)

```
enum Vis { Private, Protected, Module, ProtectedOrModule, ProtectedAndModule, Public }

fn can_access(caller: Ctx, target: Member) -> bool {
    match target.vis {
        Public  => true,
        Private => caller.type == target.owner || is_nested_in(caller.type, target.owner),
        Module  => same_module(caller.module, target.owner.module) || is_friend(caller.module, target.owner.module),
        Protected => is_subtype_chain(caller.type, target.owner) || is_enclosed_by_subtype(caller.type, target.owner),
        ProtectedOrModule  => can_access_as(Module) || can_access_as(Protected),
        ProtectedAndModule => can_access_as(Module) && can_access_as(Protected),
    }
}
```

Notas:

- A estrutura espelha `ClassLoader::CanAccess`/`CanAccessFamily` (subir pela cadeia de pais e pelos tipos envolventes).
- `is_friend` corresponde a `AssemblyOrFriendAccessAllowed`.
- O contexto de chamada é um trio, como em `AccessCheckContext`.
- Decida se o resultado é cacheado por call site (o .NET verifica na resolução, não a cada execução).
- Decida se a falha é exceção em runtime ou erro de compilação.

## Referências

- `src/coreclr/vm/clsload.cpp`: `CanAccessMethodInstantiation` (~4075), `CanAccessClass` (~4133), `CanAccess` (~4271), `CheckAccessMember` (~4350), `CanAccessFamily` (~4537).
- `src/coreclr/vm/clsload.hpp`: `AccessCheckContext` (~282), `AccessCheckOptions` (~332).
- `src/coreclr/vm/methodtablebuilder.cpp`: `TestOverRide`, comentário sobre MethodImpl e visibilidade (~4910).
- `src/coreclr/vm/assembly.cpp` (~2477), `src/coreclr/vm/comdynamic.cpp` (~857): InternalsVisibleTo / IgnoresAccessChecksTo.
- `docs/design/features/unsafeaccessors.md`.
