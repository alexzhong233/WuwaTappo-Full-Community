// 官方安装包只靠哈希是防不住投毒的：哈希由流水线自己算、自己写回锁，
// 锁被改就一起通过。这里锁住签名校验这条独立依据的行为。
import assert from 'node:assert/strict';
import test from 'node:test';
import { assertOfficialSigner, readSigner, verifyPublisher } from '../scripts/sync-upstream.mjs';

const officialSigner = `  Signer's certificate:
    Subject: /C=CN/ST=Guangdong/L=Guangzhou/O=Guangzhou Kuro Games/CN=广州库洛数界科技有限公司
    Issuer : /C=US/O=DigiCert, Inc./CN=DigiCert Trusted G4 Code Signing RSA4096 SHA384 2021 CA1
    Serial : 0A1B2C`;

const timestampSigner = `  Timestamp:
    Subject: /C=US/O=DigiCert Inc/CN=DigiCert Timestamp 2024
    Issuer : /C=US/O=DigiCert Inc/CN=DigiCert Trusted G4 RSA4096 SHA256 2023 CA1`;

const verified = body => `Signature verification: ok\n\n${body}\n  Signature verification: ok\n`;

test('reads every subject and issuer instead of only the first one', () => {
  const { subjects, issuers } = readSigner(verified(`${timestampSigner}\n${officialSigner}`));
  assert.equal(subjects.length, 2);
  assert.equal(issuers.length, 2);
  assert.ok(subjects.some(value => value.includes('CN=广州库洛数界科技有限公司')));
});

test('accepts the official signer even when a timestamp certificate comes first', () => {
  const signer = assertOfficialSigner(verified(`${timestampSigner}\n${officialSigner}`));
  assert.match(signer.subject, /广州库洛数界科技有限公司/);
  assert.match(signer.issuer, /DigiCert Trusted G4 Code Signing/);
});

test('rejects an unsigned or unsigned-by-someone-else installer', () => {
  assert.throws(() => assertOfficialSigner('Signature verification: ok'), /读不到签名主体/);
  const repacked = verified(`  Signer's certificate:
    Subject: /C=CN/O=Evil/CN=广州库洛数界科技有限公司
    Issuer : /C=CN/O=Evil/CN=Evil Code Signing CA`);
  // 主体名可以照抄，但签发 CA 抄不了。
  assert.throws(() => assertOfficialSigner(repacked), /不是官方 EV 签发/);
  const otherVendor = verified(`  Signer's certificate:
    Subject: /C=CN/O=Other/CN=Other Studio
    Issuer : /C=US/O=DigiCert, Inc./CN=DigiCert Trusted G4 Code Signing RSA4096 SHA384 2021 CA1`);
  assert.throws(() => assertOfficialSigner(otherVendor), /不是官方发行方/);
});

test('a failed osslsigncode run stops the build instead of warning', () => {
  const execute = () => ({ status: 1, stdout: '', stderr: 'Failed to verify signature' });
  assert.throws(() => verifyPublisher(Buffer.from('x'), { execute }), /签名校验失败/);
});

test('a matching signature passes and reports who signed it', () => {
  const execute = () => ({ status: 0, stdout: verified(officialSigner), stderr: '' });
  const signer = verifyPublisher(Buffer.from('x'), { execute });
  assert.match(signer.subject, /广州库洛数界科技有限公司/);
});
