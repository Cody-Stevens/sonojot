import "../../brand/brand.css";
import "../../styles.css";
import "../../layout.css";
import "../../../../shared/typography-material.css";
import "../../../../shared/typography-lab.css";
import { mountTypographyLab } from "../../../../shared/typography-lab";
import { mountTypographyMaterial } from "../../../../shared/typography-material";

const target = document.getElementById("app");
if (!target) throw new Error("The typography lab root is missing.");

mountTypographyLab(target, {
  darkTheme: "slate",
  panelClass: "studio",
  lightNote: "Cream stock, green ink — the app's light theme.",
  darkTitle: "Slate",
  darkNote: "Basalt charcoal, warm ivory ink — the app's dark theme.",
});
mountTypographyMaterial(target);
