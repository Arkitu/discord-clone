const forge = require('node-forge');

let aCle = new forge.util.ByteBuffer('12345678901234567890123456789012');
let aIv = new forge.util.ByteBuffer('1234567890123456');
let aChaine = new forge.util.ByteBuffer('1');

aCle = forge.md.md5.create().update(aCle.bytes()).digest();
aIv = aIv.length() ? forge.md.md5.create().update(aIv.bytes()).digest() : new forge.util.ByteBuffer('\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0');
aChaine = new forge.util.ByteBuffer(aChaine);
const lChiffreur = forge.cipher.createCipher('AES-CBC', aCle);
lChiffreur.start({
    iv: aIv
});
lChiffreur.update(aChaine);

console.log(lChiffreur.finish() && lChiffreur.output.toHex());