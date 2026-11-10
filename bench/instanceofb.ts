class Animal {}
class Dog extends Animal {}
class Cat extends Animal {}
let count: number = 0;
for (let i: number = 0; i < 20000; i = i + 1) {
  const a: any = (i % 2 == 0) ? new Dog() : new Cat();
  if (a instanceof Dog) { count = count + 1; }
  if (a instanceof Animal) { count = count + 1; }
}
console.log("instanceof=" + count);
