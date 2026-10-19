# sealed, abstract, override, new (shadowing)

Convenção: "(repo)" = verificado em arquivos deste repositório; "(C#/ECMA)" = conhecimento da linguagem/especificação, não verificado lendo o repo; "(inferência)" = dedução minha.

## Mecanismo no .NET

Os quatro modificadores de C# são *açúcar* sobre bits de metadados (flags de TypeDef e MethodDef) que o type loader interpreta. O runtime não conhece as palavras-chave `override` ou `new`; conhece `virtual`, `abstract`, `final`, `newslot`, `hidebysig`.

### Mapeamento (C#/ECMA, com confirmação parcial no repo)

| C# | Flags de IL/metadados |
|---|---|
| `virtual` | `mdVirtual` + `mdNewSlot` |
| `override` | `mdVirtual`, *sem* `mdNewSlot` (reusa slot do pai) |
| `sealed override` | `mdVirtual` + `mdFinal`, sem `newslot` |
| `abstract` (método) | `mdVirtual` + `mdAbstract` (+ `newslot`) |
| `abstract` (classe) | `tdAbstract` |
| `sealed` (classe) | `tdSealed` |
| `new` (método) | não muda IL de forma obrigatória: é só supressão de warning; o método é emitido como não virtual `hidebysig`, ou virtual com `newslot` |

O repo confirma os lados que o type loader consome: `IsMdVirtual`, `IsMdNewSlot`, `IsMdFinal`, `IsMdAbstract`, `IsTdSealed` em `src/coreclr/vm/methodtablebuilder.cpp`.

### override: reutilizar o slot do pai (repo)

Em `MethodTableBuilder` (`methodtablebuilder.cpp`, região ~5380-5440), para cada método:

- Se há pai e o método *não* é `newslot`, chama `LoaderFindMethodInParentClass(assinatura)`. A busca usa um hash por nome (`bmtParent.pParentMethodHash->Lookup(methodSig.GetName())`) e compara `methodSig.Equivalent(entrySig)` (~linha 231-241).
- Achou:
  - se o método do pai não é virtual: `BuildMethodTableThrowException(BFA_NONVIRT_NO_SEARCH)`;
  - se o do pai é `final`: `IDS_CLASSLOAD_MI_FINAL_DECL`;
  - senão `TestOverRide(...)` (compatibilidade de acessibilidade e assinatura) e verificação de constraints genéricas (`IDS_CLASSLOAD_CONSTRAINT_MISMATCH_ON_IMPLICIT_OVERRIDE`). O método reutiliza o slot do pai.
- Se é `newslot`: não procura no pai, recebe slot novo.

Consequência: o par "mesmo nome + mesma assinatura" é a regra de override implícito *no nível do runtime*. A linguagem C# exige a palavra `override` para evitar overrides acidentais (C#), mas o runtime sozinho faria override sem ela se o IL não tiver `newslot`.

### override explícito por MethodImpl (repo)

O IL tem `.override` (tabela MethodImpl): um método pode implementar um método virtual com *nome diferente*. Validações em `methodtablebuilder.cpp` (~4870-4950), entre elas:

- declaração (decl) precisa ser virtual (`IDS_CLASSLOAD_MI_NONVIRTUAL_DECL`);
- decl não pode ser `final` (`IDS_CLASSLOAD_MI_FINAL_DECL`);
- o corpo (impl) precisa ser virtual ou estático (`IDS_CLASSLOAD_MI_MUSTBEVIRTUAL`);
- o pai da declaração não pode ser `tdSealed` (`IDS_CLASSLOAD_MI_SEALED_DECL`);
- virtuais estáticos só em interface (`IDS_CLASSLOAD_STATICVIRTUAL`);
- "Non-static interface method body that has methodimpl should always be final" (`IDS_CLASSLOAD_MI_FINAL_IMPL`).

Isso sustenta implementação explícita de interface (`void IFoo.Bar()`) e covariância de retorno: `docs/design/features/covariant-return-methods.md` descreve que overrides com tipo de retorno mais derivado só podem ser expressos por MethodImpl, e que o loader checa compatibilidade de retorno ao fim do estágio `CLASS_LOAD_EXACTPARENTS` via `MethodTable::CanCastTo()`, com um atributo `PreserveBaseOverridesAttribute` para manter o comportamento de unificação de slots (repo).

### sealed

- Classe `sealed` (`tdSealed`): não pode ser pai. `MethodTable::IsSealed()` (`methodtable.h` ~1442). Isto permite ao JIT tratar chamadas virtuais sobre esse tipo como diretas (devirtualização) (inferência; o JIT não foi lido aqui).
- Método `sealed override` = `mdFinal`: impede novos overrides nos descendentes (verificado acima pelo erro `IDS_CLASSLOAD_MI_FINAL_DECL`).

### abstract

- Classe `abstract` (`tdAbstract`): `MethodTable::IsAbstract()`. Não pode ser instanciada (checagem na criação do objeto, C#/ECMA).
- Método `abstract`: tem slot na vtable, mas sem corpo. O doc de MethodDesc afirma que o slot "must exist for all methods, even the ones that never run like abstract methods" (`docs/design/coreclr/botr/method-descriptor.md`). O loader valida combinações inválidas, ex.: `BFA_ABSTRACT_METHOD_WITH_RVA` (`methodtablebuilder.cpp` ~2922: método abstrato com RVA de corpo).
- Em interfaces, métodos instância sem corpo são `abstract virtual`; com DIM podem ter corpo (ver arquivo 01).
- Se um tipo concreto não implementa um abstrato, a falha acontece na carga do tipo (`TypeLoadException`) ou é impedida pelo compilador (C#).

### new (hiding / shadowing)

Hiding é puramente uma questão de *resolução de nome em tempo de compilação*:

- Se `Derived.M()` é `new` (não virtual ou virtual com `newslot`), um call site que enxerga o tipo estático `Base` chama `Base.M`, e um que enxerga `Derived` chama `Derived.M`. Não há substituição de slot.
- Para métodos virtuais com `newslot`, a vtable do filho tem *dois* slots: o herdado (que ainda aponta para o corpo do pai, ou para o override intermediário) e o novo. Isto decorre diretamente do código do loader: `newslot` não procura no pai e aloca slot novo (repo, ~5398).
- `hidebysig` (C#/ECMA) diz que o ocultamento é por assinatura e não por nome; linguagens como VB podem ocultar por nome. Não verifiquei o tratamento no loader.

## Por quê

1. **Separar nome de slot.** Ao tornar "reusar slot" vs "novo slot" uma decisão explícita no metadado (`newslot`), o runtime suporta tanto linguagens que querem override implícito por nome/assinatura quanto as que querem ocultamento. O runtime é neutro de linguagem.
2. **Segurança contra quebras de versão (fragile base class).** O design de `new` vs `override` nasceu para que adicionar um método à base com o mesmo nome de um método do filho não mude silenciosamente o comportamento do filho (C#; rationale conhecida, não documentada neste repo).
3. **Otimização.** `sealed` dá ao compilador a garantia de que não há mais override: permite chamada direta e inlining (inferência).
4. **Verificação na carga.** O loader rejeita IL inválido (sobrescrever não virtual, sobrescrever final, MethodImpl em tipo selado), mantendo invariantes da vtable.

## Trade-offs

- **`override` implícito no runtime** (sem `newslot`) significa que IL gerado por outro compilador pode sobrescrever acidentalmente; a linguagem tem de se proteger.
- **`new` cria dois métodos com o mesmo nome visível** e comportamento dependente do tipo estático: fonte clássica de bugs ("por que meu método não foi chamado?").
- **`sealed` impede extensão futura**: mudar de selado para não selado é compatível na fonte, mas o inverso é uma quebra. Para bibliotecas, o padrão "selado por padrão" (como em Kotlin/Swift `final` por padrão) evita contratos acidentais.
- **Abstract + slot sem corpo** cria entradas de vtable que apontam para stubs de erro/pré-código; custo pequeno.
- **MethodImpl é poderoso e complexo**: permite renomear, covariância e implementação explícita, ao custo de um conjunto grande de regras de validação (as ~10 mensagens de erro acima).

## Como aplicar na sua linguagem

1. **Escolha o padrão de extensibilidade.** Recomendação defensável: métodos são não virtuais por padrão; classes são seladas por padrão ou, ao menos, `virtual` e `open` explícitos. O .NET fez o contrário para classes (abertas por padrão) e o mesmo que você para métodos (não virtual por padrão em C#).
2. **Exija marca explícita de override** e erro quando uma assinatura coincide com um virtual do pai sem a marca. Resolve o problema de override acidental que o runtime .NET deixa para a linguagem.
3. **Decida se existe shadowing.** Opções: (a) proibir (erro de nome duplicado em hierarquia); (b) permitir só com marca `new`/`shadow` e gerar `newslot`. Se proibir, sua vtable fica mais simples: um nome+assinatura = um slot.
4. **Modele `final` no método e no tipo** e verifique no carregamento/compilação, não só no front-end, se o binário pode ser composto de várias fontes (como o IL pode vir de qualquer compilador).
5. **Abstract**: reserve o slot na vtable (para manter índice estável) e faça a falha de instanciação acontecer cedo (na criação do objeto ou na carga do tipo).
6. **Planeje covariância de retorno** se quiser: o .NET precisou de MethodImpl, checagem tardia (`CLASS_LOAD_EXACTPARENTS`) e um atributo para preservar unificação de slots. Se sua linguagem a oferece desde o início, projete a assinatura de override com retorno `<:` e o slot compartilhado.
7. **Defina a ordem de verificações no loader** para dar mensagens claras: pai não virtual, pai final, assinatura incompatível, restrições genéricas.

Tabela de decisão mínima para sua linguagem:

```
declaração            slot na vtable       pode ser sobrescrito?
-----------------------------------------------------------------
fn f()                nenhum (direto)      não
virtual fn f()        novo                 sim
override fn f()       herdado              sim, salvo se final
final override fn f() herdado              não
abstract fn f()       novo, sem corpo      obrigatório nas concretas
shadow fn f()         novo (oculta)        conforme os flags acima
```

## Resumo dos fatos confirmados

- `newslot` faz o loader pular a busca no pai (`!IsMdNewSlot` guarda a busca).
- Sem `newslot`, o loader procura por nome + assinatura no pai (`LoaderFindMethodInParentClass`).
- Pai não virtual: erro `BFA_NONVIRT_NO_SEARCH`.
- Pai `final`: erro `IDS_CLASSLOAD_MI_FINAL_DECL`.
- Compatibilidade de override: `TestOverRide`; constraints genéricas: `IDS_CLASSLOAD_CONSTRAINT_MISMATCH_ON_IMPLICIT_OVERRIDE`.
- MethodImpl: decl virtual, decl não final, pai da decl não selado, impl virtual ou estático.
- Corpo de interface com MethodImpl deve ser `final`.
- Covariância de retorno só via MethodImpl; verificada ao fim de `CLASS_LOAD_EXACTPARENTS`.

## Matriz de combinações (IL) e o que o loader faz

| virtual | newslot | final | abstract | Efeito |
|---|---|---|---|---|
| não | n/a | n/a | n/a | método comum; sem slot virtual |
| sim | sim | não | não | novo slot virtual |
| sim | não | não | não | override do pai (erro se pai não virtual/final) |
| sim | não | sim | não | override final |
| sim | sim | não | sim | novo slot sem corpo |

(Derivada das checagens citadas; combinações adicionais, como re-abstração, não foram verificadas aqui.)

## Exemplos conceituais (C#)

- `class A { public virtual void F(){} }`, `class B : A { public override void F(){} }`: `B.F` reutiliza o slot de `A.F`.
- `class C : A { public new virtual void F(){} }`: `C.F` ganha slot novo; `((A)c).F()` chama `A.F`, `c.F()` chama `C.F`.
- `sealed class D : A { public override void F(){} }`: nada pode herdar de `D`; chamadas virtuais sobre `D` podem ser devirtualizadas pelo JIT (inferência).
- `abstract class E { public abstract void G(); }`: `G` tem slot, sem corpo; `new E()` é erro.

## Perguntas de design

- Seus overrides precisam de palavra-chave? (recomendação: sim)
- Seu `new`/shadowing cria slot novo ou apenas oculta o nome? Defina e teste a chamada pelo tipo base.
- Você oferece covariância de retorno? Defina em que fase do carregamento é verificada.
- `final` por padrão em classes? Prefira marcar `open`/`virtual` explicitamente.
- Como o compilador reage a assinaturas "quase iguais" (mesmo nome, parâmetros diferentes)? O runtime usa igualdade exata de assinatura (`Equivalent`).

## Referências

- `src/coreclr/vm/methodtablebuilder.cpp`: ~2895-2990 (flags de método, virtual estático, abstract), ~4870-4950 (validação MethodImpl), ~5380-5440 (override vs newslot, final), `LoaderFindMethodInParentClass` (~200-250).
- `src/coreclr/vm/methodtable.h`: `IsSealed()`, `IsAbstract()`; `src/coreclr/vm/class.h`: `IsSealed`, `IsAbstract`.
- `docs/design/features/covariant-return-methods.md`.
- `docs/design/specs/Ecma-335-Augments.md`: seções "II.10.3 Introducing and overriding virtual methods" e "II.10.3.2 The .override directive".
- `docs/design/coreclr/botr/method-descriptor.md` (slot existe mesmo para abstratos).
