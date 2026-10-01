import { describe, expect, it } from "vitest";
import instance from "./instance";
import library from "./library";
import realm from "./realm";
describe("deletion consequence copy", () => {
  for (const lang of ["zh", "en"] as const) {
    it(`${lang} explains trash fallback and backup across destructive flows`, () => {
      const messages = ["deleteFileBody", "deleteModBody", "deleteWorldBody", "deleteInstanceBodyDetail", "deleteInstanceBodyRow", "updateBody"].map(key => instance[lang][key]);
      messages.push(library[lang].bulkDeleteBody, realm[lang].removeExtrasHint);
      for (const message of messages) {
        expect(message).toMatch(lang === "zh" ? /回收站/ : /trash/);
        expect(message).toMatch(lang === "zh" ? /若失败.*永久删除/ : /if that fails.*permanent|permanently deleted if that fails/);
        expect(message).toMatch(lang === "zh" ? /备份/ : /Back up/);
      }
    });
    it(`${lang} does not claim every removed modpack file reached the trash`, () => {
      expect(instance[lang].updateRemoved).toMatch(lang === "zh" ? /已尝试/ : /Attempted/);
      expect(instance[lang].updateRemoved).not.toMatch(/回收站|trash|recycle bin/);
    });
  }
});
