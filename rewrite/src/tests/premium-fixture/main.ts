import { mount } from "svelte";
import PremiumFixture from "./PremiumFixture.svelte";
import "../../brand/brand.css";
import "../../styles.css";
import "../../layout.css";

const target = document.getElementById("app");
if (!target) throw new Error("The spacing fixture root is missing.");
mount(PremiumFixture, { target });
