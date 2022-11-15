import {AfterViewInit, Component, Input, OnInit} from "@angular/core";
import {MousePosition, UserInteractionService} from "../services/user-interaction.service";
import {DesktopSize, ResizeEvent, ServerBridgeService} from "../services/server-bridge.service";
import {SessionService} from "../services/session.service";
import {Session} from "../../models/session";
import {count, throttleTime} from "rxjs";

@Component({
  selector: 'app-screen-renderer',
  templateUrl: 'screen-renderer.component.html',
  styleUrls: ['screen-renderer.component.scss']
})
export class ScreenRendererComponent implements OnInit, AfterViewInit {
  @Input() scale: 'full' | 'fit' = 'full';

  currentSession: Session;

  canvas: HTMLCanvasElement;
  canvasCtx: any;

  constructor(private serverService: ServerBridgeService, private userInteractionService: UserInteractionService, private sessionService: SessionService) {
  }

  ngOnInit() {
    this.serverService.init();
  }

  ngAfterViewInit() {
    this.canvas = document.getElementById("renderer") as HTMLCanvasElement;
    this.canvasCtx = this.canvas?.getContext("2d", { alpha: false });

    this.sessionService.currentSession$.subscribe(session => {
      this.currentSession = session;
      this.canvas.width = session.desktopSize.width;
      this.canvas.height = session.desktopSize.height;
    });

    this.serverService.resize.subscribe((desktopSize: ResizeEvent) => {
      this.canvas.width = desktopSize.desktop_size.width;
      this.canvas.height = desktopSize.desktop_size.height;
    });
    this.serverService.updateImage.pipe(throttleTime(1000 / 60)).subscribe(({pixels, infos}) => {
      this.draw(pixels, infos);
    })
  }

  getMousePos(evt: any) {
    const rect = this.canvas.getBoundingClientRect(),
      scaleX = this.canvas.width / rect.width,
      scaleY = this.canvas.height / rect.height;

    const coord: MousePosition = {
      x: Math.round((evt.clientX - rect.left) * scaleX),
      y: Math.round((evt.clientY - rect.top) * scaleY)
    }

    this.userInteractionService.setMousePosition(coord);
  }

  setMouseState(state: number) {
    this.userInteractionService.setMouseLeftClickState(state);
  }

  async draw(bytesArray: any, imageInfo: any) {
    const pixels = new Uint8ClampedArray(await bytesArray);
    const imageData = new ImageData(pixels, imageInfo.width, imageInfo.height);
    this.canvasCtx?.putImageData(imageData, imageInfo.left, imageInfo.top);
  }

}
