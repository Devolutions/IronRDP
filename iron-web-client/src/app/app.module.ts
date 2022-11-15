import {NgModule} from '@angular/core';
import {BrowserModule} from '@angular/platform-browser';
import {AppComponent} from './app.component';
import {BrowserAnimationsModule} from '@angular/platform-browser/animations';
import {TabsComponent} from './tabs/tabs.component';
import {MatTabsModule} from "@angular/material/tabs";
import {AddTabComponent} from './add-tab/add-tab.component';
import {MatIconModule} from "@angular/material/icon";
import {AppRoutingModule} from "./app.routing.module";
import {SplashscreenComponent} from "./splash-screen/splashscreen.component";
import {RouterModule} from "@angular/router";
import {ServerBridgeService} from "./services/server-bridge.service";
import {environment} from "../environments/environment";
import {TauriBridgeService} from "./services/tauri-bridge.service";
import {WasmBridgeService} from "./services/wasm-bridge.service";
import {UserInteractionService} from "./services/user-interaction.service";
import {MatProgressBarModule} from '@angular/material/progress-bar';

@NgModule({
  declarations: [
    AppComponent,
    TabsComponent,
    AddTabComponent,
    SplashscreenComponent
  ],
  imports: [
    BrowserModule,
    AppRoutingModule,
    RouterModule,
    BrowserAnimationsModule,
    MatIconModule,
    MatTabsModule,
    MatProgressBarModule
  ],
  bootstrap: [AppComponent],
  providers: [
    UserInteractionService,
    {
      provide: ServerBridgeService,
      useFactory: () => {
        return environment.tauri ? new TauriBridgeService() : new WasmBridgeService();
      }
    },
  ]
})

export class AppModule {
}
