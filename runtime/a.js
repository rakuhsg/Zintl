const text = await Zintl.readFile('fs://a.js', 'utf8');
console.log(text);
const atext = await Zintl.readFile('fs://Cargo.toml', 'utf8');
console.log(atext);
