import {NgModule} from "@angular/core";
import {SessionComponent} from "./session.component";
import {CommonModule} from "@angular/common";
import {SessionRoutingModule} from "./session.routing.module";
import {LoginComponent} from "../login/login.component";
import {ScreenRendererComponent} from "../screen-renderer/screen-renderer.component";
import {MatFormFieldModule} from "@angular/material/form-field";
import {MatCardModule} from "@angular/material/card";
import {MatButtonModule} from "@angular/material/button";
import {MatInputModule} from "@angular/material/input";
import {FormsModule, ReactiveFormsModule} from "@angular/forms";
import {MatSnackBarModule} from "@angular/material/snack-bar";
import {MatTabsModule} from "@angular/material/tabs";
import {MatIconModule} from "@angular/material/icon";
import {ServerBridgeService} from "../services/server-bridge.service";
import {environment} from "../../environments/environment";
import {TauriBridgeService} from "../services/tauri-bridge.service";
import {WasmBridgeService} from "../services/wasm-bridge.service";
import {UserInteractionService} from "../services/user-interaction.service";

@NgModule({
  imports: [
    CommonModule,
    SessionRoutingModule,
    MatButtonModule,
    MatFormFieldModule,
    MatInputModule,
    MatCardModule,
    FormsModule,
    ReactiveFormsModule,
    MatSnackBarModule,
    MatTabsModule,
    MatIconModule
  ],
  declarations: [
    SessionComponent,
    LoginComponent,
    ScreenRendererComponent,
  ],
})
export class SessionModule {
}
